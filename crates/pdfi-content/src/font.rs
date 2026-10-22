//! What the interpreter needs of a font: how strings split into character
//! codes, the width of each code (for advances and bounding boxes) and the
//! text it stands for (for showing decoded strings next to `Tj` / `TJ`).
//!
//! Glyph outlines and full font parsing belong to pdfi-font (M9); this reads
//! only the font dictionaries, `/ToUnicode` and embedded CMaps. Where widths
//! have to be guessed (standard 14 fonts outside ASCII, predefined CJK CMaps)
//! the font is marked `approximate`.

use std::collections::HashMap;

use pdfi_model::resolve;
use pdfi_syntax::lexer::{Lexer, Tok, Token};
use pdfi_syntax::{Dict, DictExt, Name, ObjId, Object, PdfFile};

/// Largest CMap stream read.
const MAX_CMAP: usize = 4 << 20;
/// Codes one `bfrange` / `cidrange` line may cover.
const MAX_RANGE: u32 = 1 << 16;
/// Entries read from a CMap.
const MAX_ENTRIES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontKind {
    Simple,
    Type0,
    Type3,
}

/// How a string splits into codes.
#[derive(Clone, Debug, PartialEq)]
enum Codes {
    OneByte,
    TwoBytes,
    /// Codespace ranges: (bytes, low, high).
    Ranges(Vec<(u8, u32, u32)>),
    /// Variable-length encodings of predefined CMaps, by their lead byte.
    ShiftJis,
    Euc,
    /// GBK, Big Five, UHC: a lead byte of 0x81 or more starts two bytes.
    DoubleByte,
    Utf16,
}

#[derive(Clone, Debug, Default)]
struct CidWidths {
    /// (first CID, last CID, width) and single entries as first == last.
    ranges: Vec<(u32, u32, f64)>,
    default: f64,
}

impl CidWidths {
    fn get(&self, cid: u32) -> f64 {
        let k = self.ranges.partition_point(|r| r.1 < cid);
        match self.ranges.get(k) {
            Some(&(lo, _, w)) if lo <= cid => w,
            _ => self.default,
        }
    }
}

#[derive(Debug)]
pub struct FontInfo {
    pub id: Option<ObjId>,
    pub base_font: Option<String>,
    pub subtype: Option<String>,
    pub kind: FontKind,
    pub vertical: bool,
    /// Glyph space units to text space (0.001 except for Type 3 fonts).
    pub scale: f64,
    /// Above and below the baseline, in text space per unit of font size.
    pub ascent: f64,
    pub descent: f64,
    /// Some widths are guesses.
    pub approximate: bool,
    codes: Codes,
    /// Simple fonts: widths by code (glyph space units).
    widths: Vec<f64>,
    first_char: u32,
    missing_width: f64,
    /// Type 0: code → CID (`None`: identity).
    cids: Option<HashMap<(u8, u32), u32>>,
    cid_widths: CidWidths,
    to_unicode: HashMap<(u8, u32), String>,
    /// Simple fonts without `/ToUnicode`: text by code.
    encoding: Option<Vec<Option<char>>>,
    utf16_codes: bool,
}

/// One character code of a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub code: u32,
    pub len: u8,
}

impl FontInfo {
    /// Split `s` into character codes.
    pub fn codes(&self, s: &[u8]) -> Vec<Code> {
        let mut out = Vec::with_capacity(s.len());
        let mut i = 0;
        while i < s.len() {
            let n = self.code_len(&s[i..]).clamp(1, s.len() - i);
            let code = s[i..i + n].iter().fold(0u32, |a, &b| a << 8 | u32::from(b));
            out.push(Code { code, len: n as u8 });
            i += n;
        }
        out
    }

    fn code_len(&self, s: &[u8]) -> usize {
        let b = s[0];
        match &self.codes {
            Codes::OneByte => 1,
            Codes::TwoBytes => 2,
            Codes::Ranges(r) => {
                for n in 1..=4u8 {
                    if s.len() < n as usize {
                        break;
                    }
                    let v = s[..n as usize]
                        .iter()
                        .fold(0u32, |a, &b| a << 8 | u32::from(b));
                    if r.iter()
                        .any(|&(len, lo, hi)| len == n && lo <= v && v <= hi)
                    {
                        return n as usize;
                    }
                }
                // Not in any range: take the shortest length the ranges use.
                r.iter().map(|x| x.0 as usize).min().unwrap_or(1)
            }
            Codes::ShiftJis => {
                if (0x81..=0x9f).contains(&b) || (0xe0..=0xfc).contains(&b) {
                    2
                } else {
                    1
                }
            }
            Codes::Euc => match b {
                0x8f => 3,
                0x8e | 0xa1..=0xfe => 2,
                _ => 1,
            },
            Codes::DoubleByte => {
                if b >= 0x81 {
                    2
                } else {
                    1
                }
            }
            Codes::Utf16 => {
                if (0xd8..=0xdb).contains(&b) {
                    4
                } else {
                    2
                }
            }
        }
    }

    /// Horizontal (or, for vertical fonts, vertical) advance of `c` in text
    /// space per unit of font size. Vertical advances are negative (down).
    pub fn advance(&self, c: Code) -> f64 {
        match self.kind {
            FontKind::Type0 => {
                let cid = match &self.cids {
                    Some(m) => m.get(&(c.len, c.code)).copied().unwrap_or(0),
                    None => c.code,
                };
                if self.vertical {
                    // /W2 is not read: the default vertical metrics.
                    -1.0
                } else {
                    self.cid_widths.get(cid) * self.scale
                }
            }
            _ => {
                let w = c
                    .code
                    .checked_sub(self.first_char)
                    .and_then(|i| self.widths.get(i as usize).copied())
                    .unwrap_or(self.missing_width);
                w * self.scale
            }
        }
    }

    /// The text `c` stands for, when it can be told.
    pub fn text(&self, c: Code) -> Option<String> {
        if let Some(t) = self.to_unicode.get(&(c.len, c.code)) {
            return Some(t.clone());
        }
        if self.utf16_codes {
            let units = if c.len == 4 {
                vec![(c.code >> 16) as u16, c.code as u16]
            } else {
                vec![c.code as u16]
            };
            return Some(String::from_utf16_lossy(&units));
        }
        let e = self.encoding.as_ref()?;
        e.get(c.code as usize).copied().flatten().map(String::from)
    }

    /// Whether word spacing applies: single-byte code 32.
    pub fn is_space(&self, c: Code) -> bool {
        c.len == 1 && c.code == 32
    }

    /// A font object that is not a font dictionary.
    pub fn unknown() -> Self {
        FontInfo {
            id: None,
            base_font: None,
            subtype: None,
            kind: FontKind::Simple,
            vertical: false,
            scale: 0.001,
            ascent: 0.8,
            descent: -0.2,
            approximate: true,
            codes: Codes::OneByte,
            widths: Vec::new(),
            first_char: 0,
            missing_width: 500.0,
            cids: None,
            cid_widths: CidWidths::default(),
            to_unicode: HashMap::new(),
            encoding: None,
            utf16_codes: false,
        }
    }
}

fn num(file: &PdfFile, o: &Object) -> Option<f64> {
    resolve(file, o).1.as_f64()
}

fn numbers(file: &PdfFile, o: &Object) -> Vec<f64> {
    resolve(file, o)
        .1
        .as_array()
        .map(|a| a.iter().map(|n| num(file, &n.obj).unwrap_or(0.0)).collect())
        .unwrap_or_default()
}

fn name_of(file: &PdfFile, d: &Dict, key: &str) -> Option<Name> {
    d.get_key(key)
        .and_then(|o| resolve(file, o).1.as_name().cloned())
}

/// Read a font dictionary.
pub fn load(file: &PdfFile, id: Option<ObjId>, d: &Dict) -> FontInfo {
    let mut f = FontInfo::unknown();
    f.id = id;
    f.approximate = false;
    f.missing_width = 0.0;
    f.base_font = name_of(file, d, "BaseFont").map(|n| n.as_str_lossy().into_owned());
    let subtype = name_of(file, d, "Subtype");
    f.subtype = subtype.as_ref().map(|n| n.as_str_lossy().into_owned());
    f.kind = match subtype.as_ref().map(Name::as_bytes) {
        Some(b"Type0") => FontKind::Type0,
        Some(b"Type3") => FontKind::Type3,
        _ => FontKind::Simple,
    };
    if let Some(u) = d.get_key("ToUnicode") {
        if let Some(s) = resolve(file, u).1.as_stream() {
            let cm = read_cmap(file, u, s);
            f.to_unicode = cm.unicode;
        }
    }
    match f.kind {
        FontKind::Type0 => type0(file, d, &mut f),
        FontKind::Simple | FontKind::Type3 => simple(file, d, &mut f),
    }
    f
}

fn descriptor_metrics(file: &PdfFile, fd: &Dict, f: &mut FontInfo, scale: f64) {
    let get = |k: &str| fd.get_key(k).and_then(|o| num(file, o));
    let bbox = fd
        .get_key("FontBBox")
        .map(|o| numbers(file, o))
        .unwrap_or_default();
    let (asc, desc) = (get("Ascent").unwrap_or(0.0), get("Descent").unwrap_or(0.0));
    if asc != 0.0 || desc != 0.0 {
        f.ascent = asc * scale;
        f.descent = desc * scale;
    } else if bbox.len() == 4 && bbox[3] > bbox[1] {
        f.ascent = bbox[3] * scale;
        f.descent = bbox[1] * scale;
    }
}

fn simple(file: &PdfFile, d: &Dict, f: &mut FontInfo) {
    f.first_char = d
        .get_key("FirstChar")
        .and_then(|o| resolve(file, o).1.as_i64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    f.widths = d
        .get_key("Widths")
        .map(|o| numbers(file, o))
        .unwrap_or_default();
    let fd = d.get_key("FontDescriptor").map(|o| resolve(file, o).1);
    if let Some(fd) = fd.as_ref().and_then(|r| r.as_dict()) {
        f.missing_width = fd
            .get_key("MissingWidth")
            .and_then(|o| num(file, o))
            .unwrap_or(0.0);
    }
    if f.kind == FontKind::Type3 {
        let m = d
            .get_key("FontMatrix")
            .map(|o| numbers(file, o))
            .unwrap_or_default();
        let m = if m.len() == 6 {
            m
        } else {
            vec![0.001, 0.0, 0.0, 0.001, 0.0, 0.0]
        };
        f.scale = m[0];
        let bbox = d
            .get_key("FontBBox")
            .map(|o| numbers(file, o))
            .unwrap_or_default();
        if bbox.len() == 4 && bbox[3] > bbox[1] {
            f.ascent = bbox[3] * m[3];
            f.descent = bbox[1] * m[3];
        }
    } else if let Some(fd) = fd.as_ref().and_then(|r| r.as_dict()) {
        descriptor_metrics(file, fd, f, 0.001);
    }
    let std = f.base_font.as_deref().and_then(standard_font);
    if f.widths.is_empty() {
        match std {
            Some((Some(table), exact)) => {
                f.first_char = 32;
                f.widths = table.iter().map(|&w| f64::from(w)).collect();
                f.missing_width = 500.0;
                // Exact for ASCII; other codes get the fallback width.
                f.approximate = !exact;
            }
            Some((None, _)) => {
                // Courier: every glyph is 600 wide.
                f.first_char = 0;
                f.widths = vec![600.0; 256];
            }
            None => {
                f.missing_width = 500.0;
                f.approximate = true;
            }
        }
    }
    // Text without /ToUnicode, from the encoding.
    let symbolic = matches!(
        f.base_font.as_deref().map(strip_subset),
        Some("Symbol" | "ZapfDingbats")
    );
    let enc = d.get_key("Encoding").map(|o| resolve(file, o).1);
    let mut table: Option<Vec<Option<char>>> = None;
    let mut differences = None;
    match enc.as_deref() {
        Some(Object::Name(n)) => table = base_encoding(n.as_bytes()),
        Some(o) => {
            if let Some(ed) = o.as_dict() {
                table = name_of(file, ed, "BaseEncoding").and_then(|n| base_encoding(n.as_bytes()));
                differences = ed.get_key("Differences").cloned();
            }
        }
        None => {}
    }
    if table.is_none() && !symbolic {
        table = Some(match f.subtype.as_deref() {
            Some("TrueType") => win_ansi(),
            _ => standard(),
        });
    }
    if let Some(diff) = differences {
        let t = table.get_or_insert_with(|| vec![None; 256]);
        if let Some(a) = resolve(file, &diff).1.as_array() {
            let mut code = 0usize;
            for n in a {
                match &n.obj {
                    Object::Integer(i) => code = (*i).clamp(0, 255) as usize,
                    Object::Name(g) => {
                        if code < 256 {
                            t[code] = glyph_char(g.as_bytes());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    f.encoding = table;
}

fn type0(file: &PdfFile, d: &Dict, f: &mut FontInfo) {
    f.codes = Codes::TwoBytes;
    let enc = d.get_key("Encoding");
    match enc.map(|o| resolve(file, o).1).as_deref() {
        Some(Object::Name(n)) => {
            let s = n.as_bytes();
            f.vertical = s.ends_with(b"-V");
            if s != b"Identity-H" && s != b"Identity-V" {
                let name = n.as_str_lossy();
                f.codes = predefined_codes(&name);
                f.utf16_codes = name.contains("UCS2") || name.contains("UTF16");
                // Codes are not CIDs, and the mapping is not known here.
                f.cids = Some(HashMap::new());
                f.approximate = true;
            }
        }
        Some(Object::Stream(s)) => {
            let cm = read_cmap(file, enc.unwrap_or(&Object::Null), s);
            f.vertical = cm.vertical
                || s.dict
                    .get_key("WMode")
                    .and_then(|o| resolve(file, o).1.as_i64())
                    == Some(1);
            if !cm.codespace.is_empty() {
                f.codes = Codes::Ranges(cm.codespace);
            }
            f.cids = Some(cm.cids);
        }
        _ => {}
    }
    let desc = d
        .get_key("DescendantFonts")
        .map(|o| resolve(file, o).1)
        .and_then(|a| a.as_array().and_then(|a| a.first().map(|n| n.obj.clone())));
    let Some(desc) = desc else {
        f.approximate = true;
        f.cid_widths.default = 1000.0;
        return;
    };
    let (_, dr) = resolve(file, &desc);
    let Some(dd) = dr.as_dict() else {
        f.approximate = true;
        f.cid_widths.default = 1000.0;
        return;
    };
    f.cid_widths.default = dd
        .get_key("DW")
        .and_then(|o| num(file, o))
        .unwrap_or(1000.0);
    if let Some(w) = dd.get_key("W").map(|o| resolve(file, o).1) {
        if let Some(a) = w.as_array() {
            let mut i = 0;
            while i < a.len() {
                let Some(first) = num(file, &a[i].obj) else {
                    break;
                };
                let first = first.max(0.0) as u32;
                match a.get(i + 1).map(|n| resolve(file, &n.obj).1) {
                    Some(r) if r.as_array().is_some() => {
                        for (k, n) in r.as_array().unwrap_or(&[]).iter().enumerate() {
                            let w = num(file, &n.obj).unwrap_or(f.cid_widths.default);
                            let c = first.saturating_add(k as u32);
                            f.cid_widths.ranges.push((c, c, w));
                        }
                        i += 2;
                    }
                    Some(r) => {
                        let last = r.as_f64().unwrap_or(0.0).max(0.0) as u32;
                        let w = a
                            .get(i + 2)
                            .and_then(|n| num(file, &n.obj))
                            .unwrap_or(f.cid_widths.default);
                        f.cid_widths.ranges.push((first, last.max(first), w));
                        i += 3;
                    }
                    None => break,
                }
                if f.cid_widths.ranges.len() > MAX_ENTRIES {
                    break;
                }
            }
        }
    }
    f.cid_widths.ranges.sort_by_key(|r| r.0);
    if let Some(fd) = dd.get_key("FontDescriptor").map(|o| resolve(file, o).1) {
        if let Some(fd) = fd.as_dict() {
            descriptor_metrics(file, fd, f, 0.001);
        }
    }
}

/// Code lengths of a predefined CMap, by the family its name tells.
fn predefined_codes(name: &str) -> Codes {
    if name.contains("UCS2") {
        Codes::TwoBytes
    } else if name.contains("UTF16") {
        Codes::Utf16
    } else if name.contains("RKSJ") {
        Codes::ShiftJis
    } else if name.contains("EUC") {
        Codes::Euc
    } else if [
        "GBK", "GBpc", "B5pc", "ETen", "HKscs", "UHC", "KSCpc", "GBT",
    ]
    .iter()
    .any(|p| name.contains(p))
    {
        Codes::DoubleByte
    } else {
        Codes::TwoBytes
    }
}

#[derive(Default)]
struct CMap {
    codespace: Vec<(u8, u32, u32)>,
    unicode: HashMap<(u8, u32), String>,
    cids: HashMap<(u8, u32), u32>,
    vertical: bool,
}

fn code_of(b: &[u8]) -> Option<(u8, u32)> {
    (1..=4).contains(&b.len()).then(|| {
        (
            b.len() as u8,
            b.iter().fold(0u32, |a, &x| a << 8 | u32::from(x)),
        )
    })
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks(2)
        .map(|c| u16::from(c[0]) << 8 | u16::from(*c.get(1).unwrap_or(&0)))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Add `k` to the last UTF-16 unit of `dst` (`bfrange` with a string).
fn bump(dst: &[u8], k: u32) -> String {
    let mut units: Vec<u16> = dst
        .chunks(2)
        .map(|c| u16::from(c[0]) << 8 | u16::from(*c.get(1).unwrap_or(&0)))
        .collect();
    if let Some(last) = units.last_mut() {
        *last = last.wrapping_add(k as u16);
    }
    String::from_utf16_lossy(&units)
}

/// The next token if it is a string.
fn next_str(lex: &mut Lexer<'_>) -> Option<Vec<u8>> {
    match lex.peek() {
        Ok(Token {
            tok: Tok::Str(s), ..
        }) => {
            let _ = lex.next_token();
            Some(s.bytes)
        }
        _ => None,
    }
}

fn read_cmap(file: &PdfFile, o: &Object, s: &pdfi_syntax::Stream) -> CMap {
    let id = o.as_ref_id().unwrap_or(ObjId::new(0, 0));
    let data = file.decode_stream(id, s, Some(MAX_CMAP)).data;
    let mut cm = CMap::default();
    if s.dict.get_key("WMode").and_then(Object::as_i64) == Some(1) {
        cm.vertical = true;
    }
    let mut lex = Lexer::new_complete(&data, 0);
    let mut prev: Vec<Tok> = Vec::new();
    let count = |cm: &CMap| cm.unicode.len() + cm.cids.len();
    while let Ok(t) = lex.next_token() {
        let Tok::Keyword(k) = t.tok else {
            prev.push(t.tok);
            if prev.len() > 4 {
                prev.remove(0);
            }
            continue;
        };
        match k {
            b"begincodespacerange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    if let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) {
                        cm.codespace.push((n, l, h));
                    }
                }
            }
            b"beginbfchar" => {
                while let Some(src) = next_str(&mut lex) {
                    let Ok(dst) = lex.next_token() else { break };
                    let Some(code) = code_of(&src) else { continue };
                    match dst.tok {
                        Tok::Str(d) => {
                            cm.unicode.insert(code, utf16(&d.bytes));
                        }
                        Tok::Name(n) => {
                            if let Some(c) = glyph_char(&n) {
                                cm.unicode.insert(code, c.into());
                            }
                        }
                        _ => break,
                    }
                    if count(&cm) > MAX_ENTRIES {
                        break;
                    }
                }
            }
            b"beginbfrange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) else {
                        break;
                    };
                    let h = h.min(l.saturating_add(MAX_RANGE));
                    let Ok(dst) = lex.next_token() else { break };
                    match dst.tok {
                        Tok::Str(d) => {
                            for (k, c) in (l..=h).enumerate() {
                                cm.unicode.insert((n, c), bump(&d.bytes, k as u32));
                            }
                        }
                        Tok::ArrayOpen => {
                            let mut c = l;
                            while let Some(d) = next_str(&mut lex) {
                                if c <= h {
                                    cm.unicode.insert((n, c), utf16(&d));
                                }
                                c = c.saturating_add(1);
                            }
                            // The closing bracket.
                            let _ = lex.next_token();
                        }
                        _ => break,
                    }
                    if count(&cm) > MAX_ENTRIES {
                        break;
                    }
                }
            }
            b"begincidrange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    let Ok(Token {
                        tok: Tok::Int(cid), ..
                    }) = lex.next_token()
                    else {
                        break;
                    };
                    let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) else {
                        break;
                    };
                    let h = h.min(l.saturating_add(MAX_RANGE));
                    for (k, code) in (l..=h).enumerate() {
                        cm.cids
                            .insert((n, code), (cid.max(0) as u32).saturating_add(k as u32));
                    }
                    if count(&cm) > MAX_ENTRIES {
                        break;
                    }
                }
            }
            b"begincidchar" => {
                while let Some(src) = next_str(&mut lex) {
                    let Ok(Token {
                        tok: Tok::Int(cid), ..
                    }) = lex.next_token()
                    else {
                        break;
                    };
                    if let Some(code) = code_of(&src) {
                        cm.cids.insert(code, cid.max(0) as u32);
                    }
                    if count(&cm) > MAX_ENTRIES {
                        break;
                    }
                }
            }
            b"def" => {
                if let [.., Tok::Name(n), Tok::Int(1)] = prev.as_slice() {
                    if n == b"WMode" {
                        cm.vertical = true;
                    }
                }
            }
            _ => {}
        }
        prev.clear();
    }
    cm
}

// ------------------------------------------------------------- encodings

fn strip_subset(s: &str) -> &str {
    match s.split_once('+') {
        Some((p, rest)) if p.len() == 6 && p.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => s,
    }
}

/// Widths of a standard 14 font (or a common alias) for codes 32..=126,
/// and whether they are exact there. `None` table: Courier (600 throughout).
fn standard_font(base: &str) -> Option<(Option<&'static [u16; 95]>, bool)> {
    let s = strip_subset(base)
        .replace([' ', '-', ','], "")
        .to_ascii_lowercase();
    let bold = s.contains("bold");
    let italic = s.contains("italic") || s.contains("oblique");
    if s.starts_with("courier") {
        return Some((None, true));
    }
    if s.starts_with("helvetica") || s.starts_with("arial") {
        return Some((Some(if bold { &HELVETICA_BOLD } else { &HELVETICA }), true));
    }
    if s.starts_with("times") {
        return Some((Some(if bold { &TIMES_BOLD } else { &TIMES }), !italic));
    }
    None
}

fn base_encoding(n: &[u8]) -> Option<Vec<Option<char>>> {
    match n {
        b"WinAnsiEncoding" => Some(win_ansi()),
        b"MacRomanEncoding" => Some(mac_roman()),
        b"StandardEncoding" | b"MacExpertEncoding" => Some(standard()),
        _ => None,
    }
}

fn ascii() -> Vec<Option<char>> {
    (0..256u32)
        .map(|c| (0x20..0x7f).contains(&c).then(|| char::from(c as u8)))
        .collect()
}

fn win_ansi() -> Vec<Option<char>> {
    let mut t = ascii();
    for (i, c) in WIN_ANSI_80.chars().enumerate() {
        t[0x80 + i] = Some(c);
    }
    for c in 0xa0..=0xffu32 {
        t[c as usize] = char::from_u32(c);
    }
    t
}

fn mac_roman() -> Vec<Option<char>> {
    let mut t = ascii();
    for (i, c) in MAC_ROMAN_80.chars().enumerate() {
        t[0x80 + i] = Some(c);
    }
    t
}

fn standard() -> Vec<Option<char>> {
    let mut t = ascii();
    t[0x27] = Some('\u{2019}');
    t[0x60] = Some('\u{2018}');
    for &(code, c) in STANDARD_HIGH {
        t[code as usize] = Some(c);
    }
    t
}

/// The character a glyph name stands for (Adobe Glyph List, common names).
pub fn glyph_char(name: &[u8]) -> Option<char> {
    let n = std::str::from_utf8(name).ok()?;
    let base = n.split('.').next().unwrap_or(n);
    if let Some(hex) = base.strip_prefix("uni").filter(|h| h.len() >= 4) {
        return u32::from_str_radix(&hex[..4], 16)
            .ok()
            .and_then(char::from_u32);
    }
    if let Some(hex) = base
        .strip_prefix('u')
        .filter(|h| (4..=6).contains(&h.len()))
    {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return char::from_u32(v);
        }
    }
    if base.len() == 1 && base.as_bytes()[0].is_ascii_alphabetic() {
        return base.chars().next();
    }
    if let Some(i) = ASCII_NAMES.iter().position(|&x| x == base) {
        return char::from_u32(0x20 + i as u32);
    }
    if let Some(i) = LATIN1_NAMES.iter().position(|&x| x == base) {
        return char::from_u32(0xa1 + i as u32);
    }
    EXTRA_NAMES
        .iter()
        .find(|(x, _)| *x == base)
        .map(|&(_, c)| c)
}

const WIN_ANSI_80: &str = "€•‚ƒ„…†‡ˆ‰Š‹Œ•Ž••‘’“”•–—˜™š›œ•žŸ";
const MAC_ROMAN_80: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

const STANDARD_HIGH: &[(u8, char)] = &[
    (0xa1, '¡'),
    (0xa2, '¢'),
    (0xa3, '£'),
    (0xa4, '⁄'),
    (0xa5, '¥'),
    (0xa6, 'ƒ'),
    (0xa7, '§'),
    (0xa8, '¤'),
    (0xa9, '\''),
    (0xaa, '“'),
    (0xab, '«'),
    (0xac, '‹'),
    (0xad, '›'),
    (0xae, 'ﬁ'),
    (0xaf, 'ﬂ'),
    (0xb1, '–'),
    (0xb2, '†'),
    (0xb3, '‡'),
    (0xb4, '·'),
    (0xb6, '¶'),
    (0xb7, '•'),
    (0xb8, '‚'),
    (0xb9, '„'),
    (0xba, '”'),
    (0xbb, '»'),
    (0xbc, '…'),
    (0xbd, '‰'),
    (0xbf, '¿'),
    (0xc1, '`'),
    (0xc2, '´'),
    (0xc3, 'ˆ'),
    (0xc4, '˜'),
    (0xc5, '¯'),
    (0xc6, '˘'),
    (0xc7, '˙'),
    (0xc8, '¨'),
    (0xca, '˚'),
    (0xcb, '¸'),
    (0xcd, '˝'),
    (0xce, '˛'),
    (0xcf, 'ˇ'),
    (0xd0, '—'),
    (0xe1, 'Æ'),
    (0xe3, 'ª'),
    (0xe8, 'Ł'),
    (0xe9, 'Ø'),
    (0xea, 'Œ'),
    (0xeb, 'º'),
    (0xf1, 'æ'),
    (0xf5, 'ı'),
    (0xf8, 'ł'),
    (0xf9, 'ø'),
    (0xfa, 'œ'),
    (0xfb, 'ß'),
];

/// Glyph names of 0x20..=0x7e.
const ASCII_NAMES: [&str; 95] = [
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quotesingle",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "hyphen",
    "period",
    "slash",
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
];

/// Glyph names of 0xa1..=0xff.
const LATIN1_NAMES: [&str; 95] = [
    "exclamdown",
    "cent",
    "sterling",
    "currency",
    "yen",
    "brokenbar",
    "section",
    "dieresis",
    "copyright",
    "ordfeminine",
    "guillemotleft",
    "logicalnot",
    "uni00AD",
    "registered",
    "macron",
    "degree",
    "plusminus",
    "twosuperior",
    "threesuperior",
    "acute",
    "mu",
    "paragraph",
    "periodcentered",
    "cedilla",
    "onesuperior",
    "ordmasculine",
    "guillemotright",
    "onequarter",
    "onehalf",
    "threequarters",
    "questiondown",
    "Agrave",
    "Aacute",
    "Acircumflex",
    "Atilde",
    "Adieresis",
    "Aring",
    "AE",
    "Ccedilla",
    "Egrave",
    "Eacute",
    "Ecircumflex",
    "Edieresis",
    "Igrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Eth",
    "Ntilde",
    "Ograve",
    "Oacute",
    "Ocircumflex",
    "Otilde",
    "Odieresis",
    "multiply",
    "Oslash",
    "Ugrave",
    "Uacute",
    "Ucircumflex",
    "Udieresis",
    "Yacute",
    "Thorn",
    "germandbls",
    "agrave",
    "aacute",
    "acircumflex",
    "atilde",
    "adieresis",
    "aring",
    "ae",
    "ccedilla",
    "egrave",
    "eacute",
    "ecircumflex",
    "edieresis",
    "igrave",
    "iacute",
    "icircumflex",
    "idieresis",
    "eth",
    "ntilde",
    "ograve",
    "oacute",
    "ocircumflex",
    "otilde",
    "odieresis",
    "divide",
    "oslash",
    "ugrave",
    "uacute",
    "ucircumflex",
    "udieresis",
    "yacute",
    "thorn",
    "ydieresis",
];

const EXTRA_NAMES: &[(&str, char)] = &[
    ("quoteleft", '‘'),
    ("quoteright", '’'),
    ("quotedblleft", '“'),
    ("quotedblright", '”'),
    ("quotesinglbase", '‚'),
    ("quotedblbase", '„'),
    ("guilsinglleft", '‹'),
    ("guilsinglright", '›'),
    ("endash", '–'),
    ("emdash", '—'),
    ("bullet", '•'),
    ("ellipsis", '…'),
    ("dagger", '†'),
    ("daggerdbl", '‡'),
    ("perthousand", '‰'),
    ("trademark", '™'),
    ("Euro", '€'),
    ("florin", 'ƒ'),
    ("circumflex", 'ˆ'),
    ("tilde", '˜'),
    ("OE", 'Œ'),
    ("oe", 'œ'),
    ("Scaron", 'Š'),
    ("scaron", 'š'),
    ("Zcaron", 'Ž'),
    ("zcaron", 'ž'),
    ("Ydieresis", 'Ÿ'),
    ("fi", 'ﬁ'),
    ("fl", 'ﬂ'),
    ("ff", 'ﬀ'),
    ("ffi", 'ﬃ'),
    ("ffl", 'ﬄ'),
    ("fraction", '⁄'),
    ("dotlessi", 'ı'),
    ("Lslash", 'Ł'),
    ("lslash", 'ł'),
    ("breve", '˘'),
    ("dotaccent", '˙'),
    ("ring", '˚'),
    ("ogonek", '˛'),
    ("caron", 'ˇ'),
    ("hungarumlaut", '˝'),
    ("minus", '−'),
    ("nbspace", '\u{a0}'),
    ("sfthyphen", '\u{ad}'),
    ("space", ' '),
];

#[rustfmt::skip]
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 222, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    222, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 278, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    278, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

#[rustfmt::skip]
const TIMES: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 333, 333, 333, 500, 564, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444,
    921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722,
    556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500,
    333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500,
    500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];

#[rustfmt::skip]
const TIMES_BOLD: [u16; 95] = [
    250, 333, 555, 500, 500, 1000, 833, 333, 333, 333, 500, 570, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500,
    930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778,
    611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500,
    333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500,
    556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_complete() {
        assert_eq!(WIN_ANSI_80.chars().count(), 32);
        assert_eq!(MAC_ROMAN_80.chars().count(), 128);
        assert_eq!(glyph_char(b"Aacute"), Some('Á'));
        assert_eq!(glyph_char(b"ydieresis"), Some('ÿ'));
        assert_eq!(glyph_char(b"uni3042"), Some('あ'));
        assert_eq!(glyph_char(b"g.sc"), Some('g'));
        assert_eq!(glyph_char(b"asciitilde"), Some('~'));
    }

    #[test]
    fn standard_widths() {
        let (t, exact) = standard_font("ABCDEF+Helvetica-Bold").unwrap();
        assert!(exact);
        assert_eq!(t.unwrap()[b'A' as usize - 32], 722);
        assert!(standard_font("Courier-Oblique").unwrap().0.is_none());
        assert!(standard_font("Foo").is_none());
    }

    #[test]
    fn code_ranges() {
        let mut f = FontInfo::unknown();
        f.codes = Codes::Ranges(vec![(1, 0x00, 0x80), (2, 0x8140, 0xfefe)]);
        let c = f.codes(b"\x41\x81\x40\x42");
        assert_eq!(c.iter().map(|c| c.len).collect::<Vec<_>>(), [1, 2, 1]);
        assert_eq!(c[1].code, 0x8140);
    }
}
