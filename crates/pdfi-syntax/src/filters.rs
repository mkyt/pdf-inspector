//! Stream filters as push-based decoders chained behind an `io::Read`.
//!
//! Decoding is incremental so callers can read a prefix of a huge stream
//! without decoding all of it. Decoders are tolerant: on corrupt input the
//! output produced so far is kept, the error is recorded and the chain ends.

use std::io::{self, Read};

use flate2::{Decompress, FlushDecompress, Status};

use crate::object::{Dict, DictExt, Name, Object};

pub trait Decoder: Send {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String>;
    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String>;
    /// The decoder saw its end-of-data marker; further input is ignored.
    fn done(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterSpec {
    pub name: Name,
    pub parms: Option<Dict>,
}

/// Canonical filter name (expands the abbreviations used in inline images).
pub fn canonical_name(n: &[u8]) -> &[u8] {
    match n {
        b"AHx" => b"ASCIIHexDecode",
        b"A85" => b"ASCII85Decode",
        b"LZW" => b"LZWDecode",
        b"Fl" => b"FlateDecode",
        b"RL" => b"RunLengthDecode",
        b"CCF" => b"CCITTFaxDecode",
        b"DCT" => b"DCTDecode",
        other => other,
    }
}

/// Image codecs that are decoded by the image layer, not here.
pub fn is_image_codec(n: &[u8]) -> bool {
    matches!(
        canonical_name(n),
        b"DCTDecode" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode"
    )
}

/// Read `/Filter` and `/DecodeParms` (direct values only; resolve
/// references before calling).
pub fn filter_specs(dict: &Dict) -> Vec<FilterSpec> {
    let filter = dict.get_key("Filter").or_else(|| dict.get_key("F"));
    let parms = dict.get_key("DecodeParms").or_else(|| dict.get_key("DP"));
    let names: Vec<Name> = match filter {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a.iter().filter_map(|n| n.obj.as_name().cloned()).collect(),
        _ => Vec::new(),
    };
    let parm_at = |i: usize| -> Option<Dict> {
        match parms {
            Some(Object::Dict(d)) if i == 0 => Some(d.clone()),
            Some(Object::Array(a)) => a.get(i).and_then(|n| n.obj.as_dict().cloned()),
            _ => None,
        }
    };
    names
        .into_iter()
        .enumerate()
        .map(|(i, name)| FilterSpec {
            name,
            parms: parm_at(i),
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct Chain {
    pub decoders: Vec<Box<dyn Decoder>>,
    /// Name of the first filter that is left encoded (image codec or
    /// unsupported).
    pub stopped_at: Option<Name>,
    pub unsupported: bool,
}

impl std::fmt::Debug for dyn Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Decoder")
    }
}

pub fn build_chain(specs: &[FilterSpec]) -> Chain {
    let mut chain = Chain::default();
    for spec in specs {
        let name = canonical_name(spec.name.as_bytes());
        let parms = spec.parms.as_ref();
        match name {
            b"FlateDecode" => {
                chain.decoders.push(Box::new(FlateDecoder::new()));
                push_predictor(&mut chain.decoders, parms);
            }
            b"LZWDecode" => {
                let early = parms.and_then(|p| p.get_i64("EarlyChange")).unwrap_or(1) != 0;
                chain.decoders.push(Box::new(LzwDecoder::new(early)));
                push_predictor(&mut chain.decoders, parms);
            }
            b"ASCIIHexDecode" => chain.decoders.push(Box::new(AsciiHexDecoder::default())),
            b"ASCII85Decode" => chain.decoders.push(Box::new(Ascii85Decoder::default())),
            b"RunLengthDecode" => chain.decoders.push(Box::new(RunLengthDecoder::default())),
            // Crypt filters are applied by the security handler; Identity is a no-op.
            b"Crypt" => {}
            n => {
                chain.unsupported = !is_image_codec(n);
                chain.stopped_at = Some(Name(n.to_vec()));
                break;
            }
        }
    }
    chain
}

fn push_predictor(decoders: &mut Vec<Box<dyn Decoder>>, parms: Option<&Dict>) {
    let Some(p) = parms else { return };
    let predictor = p.get_i64("Predictor").unwrap_or(1);
    if predictor <= 1 {
        return;
    }
    let colors = p.get_i64("Colors").unwrap_or(1).clamp(1, 32) as usize;
    let bpc = p.get_i64("BitsPerComponent").unwrap_or(8).clamp(1, 16) as usize;
    let columns = p.get_i64("Columns").unwrap_or(1).clamp(1, 1 << 24) as usize;
    decoders.push(Box::new(PredictorDecoder::new(
        predictor, colors, bpc, columns,
    )));
}

/// `io::Read` adapter running `inner` through a decoder chain.
pub struct DecodeReader<R> {
    inner: R,
    stages: Vec<(Box<dyn Decoder>, bool)>, // (decoder, still live)
    pending: Vec<u8>,
    pending_pos: usize,
    inner_done: bool,
    finished: bool,
    errors: Vec<String>,
}

impl<R: Read> DecodeReader<R> {
    pub fn new(inner: R, decoders: Vec<Box<dyn Decoder>>) -> Self {
        Self {
            inner,
            stages: decoders.into_iter().map(|d| (d, true)).collect(),
            pending: Vec::new(),
            pending_pos: 0,
            inner_done: false,
            finished: false,
            errors: Vec::new(),
        }
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Push `data` through all stages. A stage that fails or reaches its
    /// end-of-data marker is finished, and so is everything downstream.
    fn push(&mut self, mut data: Vec<u8>, mut finishing: bool) -> Vec<u8> {
        for (dec, live) in &mut self.stages {
            let mut out = Vec::new();
            if *live {
                if let Err(e) = dec.feed(&data, &mut out) {
                    self.errors.push(e);
                    finishing = true;
                }
                if finishing || dec.done() {
                    if let Err(e) = dec.finish(&mut out) {
                        self.errors.push(e);
                    }
                    *live = false;
                    finishing = true;
                }
            }
            data = out;
        }
        if self.stages.first().is_some_and(|(_, live)| !live) {
            self.inner_done = true;
        }
        data
    }
}

impl<R: Read> Read for DecodeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pending_pos < self.pending.len() {
                let n = buf.len().min(self.pending.len() - self.pending_pos);
                buf[..n].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
                self.pending_pos += n;
                return Ok(n);
            }
            if self.finished {
                return Ok(0);
            }
            self.pending_pos = 0;
            if self.inner_done {
                self.pending = self.push(Vec::new(), true);
                self.finished = true;
                continue;
            }
            let mut chunk = vec![0; 64 * 1024];
            let n = self.inner.read(&mut chunk)?;
            if n == 0 {
                self.inner_done = true;
                self.pending.clear();
                continue;
            }
            chunk.truncate(n);
            self.pending = if self.stages.is_empty() {
                chunk
            } else {
                self.push(chunk, false)
            };
        }
    }
}

/// Decode a whole buffer, stopping after `limit` output bytes.
pub fn decode_all(
    data: &[u8],
    decoders: Vec<Box<dyn Decoder>>,
    limit: Option<usize>,
) -> (Vec<u8>, Vec<String>) {
    let mut r = DecodeReader::new(data, decoders);
    let mut out = Vec::new();
    let res = match limit {
        Some(l) => (&mut r).take(l as u64).read_to_end(&mut out),
        None => r.read_to_end(&mut out),
    };
    let mut errors = r.errors().to_vec();
    if let Err(e) = res {
        errors.push(e.to_string());
    }
    (out, errors)
}

// ---------------------------------------------------------------------------

pub struct FlateDecoder {
    z: Option<Decompress>,
    header: Vec<u8>,
    done: bool,
}

impl FlateDecoder {
    pub fn new() -> Self {
        Self {
            z: None,
            header: Vec::new(),
            done: false,
        }
    }
}

impl Default for FlateDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for FlateDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        if self.z.is_some() {
            return self.inflate(input, out);
        }
        // Decide between zlib and raw deflate from the first two bytes.
        self.header.extend_from_slice(input);
        if self.header.len() < 2 {
            return Ok(());
        }
        let (cmf, flg) = (self.header[0], self.header[1]);
        let zlib = cmf & 0x0f == 8 && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0;
        self.z = Some(Decompress::new(zlib));
        let buffered = std::mem::take(&mut self.header);
        self.inflate(&buffered, out)
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        if self.z.is_none() && !self.header.is_empty() {
            let h = std::mem::take(&mut self.header);
            self.z = Some(Decompress::new(true));
            self.inflate(&h, out)?;
        }
        // Drain buffered output.
        self.inflate(&[], out)
    }

    fn done(&self) -> bool {
        self.done
    }
}

impl FlateDecoder {
    fn inflate(&mut self, mut input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        let Some(z) = self.z.as_mut() else {
            return Ok(());
        };
        // Keep calling after the input is consumed: the inflater may still
        // hold buffered output.
        while !self.done {
            out.reserve((input.len() * 4).max(32 * 1024));
            let (in0, out0) = (z.total_in(), z.total_out());
            let st = z.decompress_vec(input, out, FlushDecompress::None);
            let consumed = (z.total_in() - in0) as usize;
            let produced = z.total_out() - out0;
            input = &input[consumed..];
            match st {
                Ok(Status::StreamEnd) => self.done = true,
                Ok(_) if consumed == 0 && produced == 0 => break,
                Ok(_) => {}
                Err(e) => {
                    self.done = true;
                    return Err(format!("FlateDecode: {e}"));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------

pub struct LzwDecoder {
    early: bool,
    table: Vec<(u16, u8)>, // (prefix code, last byte); prefix 0xffff = none
    code_len: u32,
    bits: u32,
    nbits: u32,
    prev: Option<u16>,
    scratch: Vec<u8>,
    done: bool,
}

const LZW_CLEAR: u16 = 256;
const LZW_EOD: u16 = 257;

impl LzwDecoder {
    pub fn new(early: bool) -> Self {
        let mut d = Self {
            early,
            table: Vec::with_capacity(4096),
            code_len: 9,
            bits: 0,
            nbits: 0,
            prev: None,
            scratch: Vec::new(),
            done: false,
        };
        d.reset();
        d
    }

    fn reset(&mut self) {
        self.table.clear();
        for b in 0..=255u16 {
            self.table.push((0xffff, b as u8));
        }
        self.table.push((0xffff, 0));
        self.table.push((0xffff, 0));
        self.code_len = 9;
        self.prev = None;
    }

    fn expand(&mut self, code: u16) {
        self.scratch.clear();
        let mut c = code;
        while c != 0xffff {
            let (p, b) = self.table[c as usize];
            self.scratch.push(b);
            c = p;
        }
        self.scratch.reverse();
    }

    fn update_len(&mut self) {
        let n = self.table.len() + usize::from(self.early);
        self.code_len = match n {
            ..512 => 9,
            512..1024 => 10,
            1024..2048 => 11,
            _ => 12,
        };
    }
}

impl Decoder for LzwDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        for &byte in input {
            if self.done {
                return Ok(());
            }
            self.bits = self.bits << 8 | u32::from(byte);
            self.nbits += 8;
            while self.nbits >= self.code_len && !self.done {
                let shift = self.nbits - self.code_len;
                let code = ((self.bits >> shift) & ((1 << self.code_len) - 1)) as u16;
                self.nbits -= self.code_len;
                self.bits &= (1 << self.nbits) - 1;
                match code {
                    LZW_CLEAR => self.reset(),
                    LZW_EOD => self.done = true,
                    _ => {
                        let next = self.table.len() as u16;
                        match self.prev {
                            None => {
                                if code > 255 {
                                    return Err("LZWDecode: invalid first code".into());
                                }
                                out.push(code as u8);
                            }
                            Some(prev) => {
                                if code < next {
                                    self.expand(code);
                                    if self.table.len() < 4096 {
                                        self.table.push((prev, self.scratch[0]));
                                    }
                                } else if code == next {
                                    self.expand(prev);
                                    let first = self.scratch[0];
                                    self.scratch.push(first);
                                    if self.table.len() < 4096 {
                                        self.table.push((prev, first));
                                    }
                                } else {
                                    return Err("LZWDecode: invalid code".into());
                                }
                                out.extend_from_slice(&self.scratch);
                            }
                        }
                        self.prev = Some(code);
                        self.update_len();
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self, _out: &mut Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn done(&self) -> bool {
        self.done
    }
}

// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct AsciiHexDecoder {
    hi: Option<u8>,
    done: bool,
}

impl Decoder for AsciiHexDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        for &b in input {
            if self.done {
                break;
            }
            let v = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                b'>' => {
                    self.done = true;
                    continue;
                }
                _ if crate::lexer::is_whitespace(b) => continue,
                _ => return Err(format!("ASCIIHexDecode: invalid byte 0x{b:02x}")),
            };
            match self.hi.take() {
                Some(h) => out.push(h << 4 | v),
                None => self.hi = Some(v),
            }
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        if let Some(h) = self.hi.take() {
            out.push(h << 4);
        }
        Ok(())
    }

    fn done(&self) -> bool {
        self.done
    }
}

#[derive(Default)]
pub struct Ascii85Decoder {
    group: [u8; 5],
    n: usize,
    tilde: bool,
    done: bool,
}

impl Ascii85Decoder {
    fn flush_group(&mut self, out: &mut Vec<u8>) {
        if self.n < 2 {
            self.n = 0;
            return;
        }
        let mut g = self.group;
        for c in g.iter_mut().skip(self.n) {
            *c = b'u' - b'!';
        }
        let v = g.iter().fold(0u64, |acc, &c| acc * 85 + u64::from(c)) as u32;
        out.extend_from_slice(&v.to_be_bytes()[..self.n - 1]);
        self.n = 0;
    }
}

impl Decoder for Ascii85Decoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        for &b in input {
            if self.done {
                break;
            }
            if self.tilde {
                if b == b'>' {
                    self.flush_group(out);
                    self.done = true;
                    continue;
                }
                return Err("ASCII85Decode: '~' not followed by '>'".into());
            }
            match b {
                b'~' => self.tilde = true,
                b'z' if self.n == 0 => out.extend_from_slice(&[0; 4]),
                b'!'..=b'u' => {
                    self.group[self.n] = b - b'!';
                    self.n += 1;
                    if self.n == 5 {
                        let v = self
                            .group
                            .iter()
                            .fold(0u64, |acc, &c| acc * 85 + u64::from(c));
                        if v > u64::from(u32::MAX) {
                            return Err("ASCII85Decode: group overflow".into());
                        }
                        out.extend_from_slice(&(v as u32).to_be_bytes());
                        self.n = 0;
                    }
                }
                _ if crate::lexer::is_whitespace(b) => {}
                _ => return Err(format!("ASCII85Decode: invalid byte 0x{b:02x}")),
            }
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        self.flush_group(out);
        Ok(())
    }

    fn done(&self) -> bool {
        self.done
    }
}

#[derive(Default)]
pub struct RunLengthDecoder {
    /// Remaining literal bytes to copy, or a pending repeat run length.
    literal: usize,
    repeat: Option<usize>,
    done: bool,
}

impl Decoder for RunLengthDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        let mut i = 0;
        while i < input.len() && !self.done {
            if self.literal > 0 {
                let n = self.literal.min(input.len() - i);
                out.extend_from_slice(&input[i..i + n]);
                self.literal -= n;
                i += n;
            } else if let Some(r) = self.repeat.take() {
                out.extend(std::iter::repeat_n(input[i], r));
                i += 1;
            } else {
                let len = input[i];
                i += 1;
                match len {
                    0..=127 => self.literal = usize::from(len) + 1,
                    128 => self.done = true,
                    _ => self.repeat = Some(257 - usize::from(len)),
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self, _out: &mut Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn done(&self) -> bool {
        self.done
    }
}

// ---------------------------------------------------------------------------

pub struct PredictorDecoder {
    predictor: i64,
    bpp: usize,
    colors: usize,
    bpc: usize,
    row_len: usize,
    prev: Vec<u8>,
    cur: Vec<u8>,
}

impl PredictorDecoder {
    pub fn new(predictor: i64, colors: usize, bpc: usize, columns: usize) -> Self {
        let row_len = (colors * bpc * columns).div_ceil(8);
        let bpp = (colors * bpc).div_ceil(8).max(1);
        Self {
            predictor,
            bpp,
            colors,
            bpc,
            row_len,
            prev: vec![0; row_len],
            cur: Vec::new(),
        }
    }

    fn png_row(&mut self, out: &mut Vec<u8>) {
        let ty = self.cur[0];
        let row = &mut self.cur[1..];
        let bpp = self.bpp;
        for i in 0..row.len() {
            let left = if i >= bpp { row[i - bpp] } else { 0 };
            let up = self.prev[i];
            let ul = if i >= bpp { self.prev[i - bpp] } else { 0 };
            let p = match ty {
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, ul),
                _ => 0,
            };
            row[i] = row[i].wrapping_add(p);
        }
        out.extend_from_slice(row);
        self.prev.copy_from_slice(row);
    }

    fn tiff_row(&mut self, out: &mut Vec<u8>) {
        let row = &mut self.cur[..];
        match self.bpc {
            8 => {
                for i in self.colors..row.len() {
                    row[i] = row[i].wrapping_add(row[i - self.colors]);
                }
            }
            16 => {
                let step = self.colors * 2;
                let mut i = step;
                while i + 1 < row.len() {
                    let a = u16::from_be_bytes([row[i - step], row[i + 1 - step]]);
                    let b = u16::from_be_bytes([row[i], row[i + 1]]);
                    row[i..i + 2].copy_from_slice(&a.wrapping_add(b).to_be_bytes());
                    i += 2;
                }
            }
            bpc => {
                // Sub-byte samples: operate on unpacked components.
                let n = row.len() * 8 / bpc;
                let mask = (1u16 << bpc) - 1;
                let get = |r: &[u8], k: usize| -> u16 {
                    let bit = k * bpc;
                    (u16::from(r[bit / 8]) >> (8 - bpc - bit % 8)) & mask
                };
                for k in self.colors..n {
                    let v = (get(row, k) + get(row, k - self.colors)) & mask;
                    let bit = k * bpc;
                    let sh = 8 - bpc - bit % 8;
                    row[bit / 8] = (row[bit / 8] & !((mask as u8) << sh)) | ((v as u8) << sh);
                }
            }
        }
        out.extend_from_slice(row);
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

impl Decoder for PredictorDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        let png = self.predictor >= 10;
        if !png && self.predictor != 2 {
            out.extend_from_slice(input);
            return Ok(());
        }
        let stride = self.row_len + usize::from(png);
        let mut input = input;
        while !input.is_empty() {
            let need = stride - self.cur.len();
            let n = need.min(input.len());
            self.cur.extend_from_slice(&input[..n]);
            input = &input[n..];
            if self.cur.len() == stride {
                if png {
                    self.png_row(out)
                } else {
                    self.tiff_row(out)
                }
                self.cur.clear();
            }
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        // Partial last row: decode what is there.
        if !self.cur.is_empty() {
            let png = self.predictor >= 10;
            let stride = self.row_len + usize::from(png);
            let have = self.cur.len();
            self.cur.resize(stride, 0);
            let mut tmp = Vec::new();
            if png {
                self.png_row(&mut tmp)
            } else {
                self.tiff_row(&mut tmp)
            }
            tmp.truncate(have - usize::from(png));
            out.extend_from_slice(&tmp);
            self.cur.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn chain(names: &[&str]) -> Vec<Box<dyn Decoder>> {
        let specs: Vec<_> = names
            .iter()
            .map(|n| FilterSpec {
                name: Name::from(*n),
                parms: None,
            })
            .collect();
        build_chain(&specs).decoders
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn flate_roundtrip_small_chunks() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let z = zlib(&data);
        // Feed one byte at a time to exercise header handling.
        let mut d = FlateDecoder::new();
        let mut out = Vec::new();
        for b in &z {
            d.feed(std::slice::from_ref(b), &mut out).unwrap();
        }
        d.finish(&mut out).unwrap();
        assert_eq!(out, data);
        let (out, errs) = decode_all(&z, chain(&["FlateDecode"]), None);
        assert!(errs.is_empty());
        assert_eq!(out, data);
    }

    #[test]
    fn flate_drains_buffered_output() {
        // Tiny input, huge output: the inflater keeps output after input runs out.
        let data = vec![0u8; 10_000_000];
        let (out, errs) = decode_all(&zlib(&data), chain(&["FlateDecode"]), None);
        assert!(errs.is_empty());
        assert_eq!(out.len(), data.len());
    }

    #[test]
    fn flate_limit_prefix() {
        let data = vec![7u8; 1_000_000];
        let (out, _) = decode_all(&zlib(&data), chain(&["FlateDecode"]), Some(10));
        assert_eq!(out, vec![7u8; 10]);
    }

    #[test]
    fn flate_truncated_keeps_prefix() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let z = zlib(&data);
        let (out, _) = decode_all(&z[..z.len() / 2], chain(&["FlateDecode"]), None);
        assert!(!out.is_empty());
        assert_eq!(&data[..out.len()], &out[..]);
    }

    #[test]
    fn ascii_hex_and_85() {
        let (out, _) = decode_all(b"48 65 6c6C6f>", chain(&["ASCIIHexDecode"]), None);
        assert_eq!(out, b"Hello");
        let (out, _) = decode_all(b"87cURD]i,\"Ebo80~>", chain(&["ASCII85Decode"]), None);
        assert_eq!(out, b"Hello World!");
        let (out, _) = decode_all(b"z~>", chain(&["A85"]), None);
        assert_eq!(out, vec![0; 4]);
    }

    #[test]
    fn run_length() {
        let (out, _) = decode_all(
            &[2, b'a', b'b', b'c', 254, b'x', 128, b'z'],
            chain(&["RunLengthDecode"]),
            None,
        );
        assert_eq!(out, b"abcxxx");
    }

    #[test]
    fn lzw_spec_example() {
        // Example from ISO 32000-1 7.4.4.2.
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let (out, errs) = decode_all(&enc, chain(&["LZWDecode"]), None);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(out, b"-----A---B");
    }

    #[test]
    fn png_predictor_up() {
        // 2 rows, 3 columns, filter type 2 (Up).
        let raw = [2, 1, 2, 3, 2, 1, 1, 1];
        let mut d = PredictorDecoder::new(12, 1, 8, 3);
        let mut out = Vec::new();
        d.feed(&raw, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn image_codec_stops_chain() {
        let specs = [
            FilterSpec {
                name: Name::from("ASCIIHexDecode"),
                parms: None,
            },
            FilterSpec {
                name: Name::from("DCTDecode"),
                parms: None,
            },
        ];
        let c = build_chain(&specs);
        assert_eq!(c.decoders.len(), 1);
        assert_eq!(c.stopped_at, Some(Name::from("DCTDecode")));
        assert!(!c.unsupported);
    }
}
