//! Predefined CMaps (`90ms-RKSJ-H`, `UniJIS-UCS2-H`, ...: code → CID) and
//! the CID → Unicode maps of the Adobe character collections, from
//! Adobe's cmap-resources and mapping-resources-pdf (BSD-3-Clause, see
//! `assets/LICENSE_CMAPS`). `scripts/gen-cmaps.py` packs them into
//! `assets/cmaps.bin` (1.3 MB, compiled in), one zlib stream per map,
//! inflated on first use and kept for the process.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

use crate::cmap::CodeKey;

static DATA: &[u8] = include_bytes!("../assets/cmaps.bin");

/// Maps a CMap may use through each other.
const MAX_DEPTH: u32 = 4;

/// A predefined CMap.
#[derive(Debug)]
pub struct Predefined {
    pub name: String,
    pub vertical: bool,
    /// The character collection (`Japan1`, `GB1`, `CNS1`, `Korea1`, ...).
    pub ordering: String,
    /// (bytes, low, high), with those of the CMap it uses.
    pub codespace: Vec<(u8, u32, u32)>,
    /// (bytes, low, high, first CID), sorted.
    ranges: Vec<(u8, u32, u32, u32)>,
    /// The CMap it uses (`usecmap`): looked up after its own ranges.
    parent: Option<Arc<Predefined>>,
}

impl Predefined {
    /// The CID of a code, if the CMap maps it.
    pub fn cid(&self, (n, code): CodeKey) -> Option<u32> {
        let k = self.ranges.partition_point(|r| (r.0, r.2) < (n, code));
        match self.ranges.get(k) {
            Some(&(rn, lo, _, cid)) if rn == n && lo <= code => Some(cid + (code - lo)),
            _ => self.parent.as_ref()?.cid((n, code)),
        }
    }
}

/// CID → Unicode of a character collection.
#[derive(Debug)]
pub struct CidText {
    /// (low CID, high CID, scalars): a range adds `cid - low` to the last.
    ranges: Vec<(u32, u32, Vec<u32>)>,
}

impl CidText {
    pub fn text(&self, cid: u32) -> Option<String> {
        let k = self.ranges.partition_point(|r| r.1 < cid);
        let (lo, _, u) = self.ranges.get(k).filter(|r| r.0 <= cid)?;
        let mut u = u.clone();
        if let Some(last) = u.last_mut() {
            *last += cid - lo;
        }
        let s: String = u.into_iter().filter_map(char::from_u32).collect();
        (!s.is_empty() && s != "\u{fffd}").then_some(s)
    }
}

/// The directory: name → (offset, length) of its zlib stream.
fn directory() -> &'static HashMap<&'static str, (usize, usize)> {
    static DIR: OnceLock<HashMap<&'static str, (usize, usize)>> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut r = Reader { d: DATA, p: 8 };
        let mut dir = HashMap::new();
        if !DATA.starts_with(b"PDFICMAP") {
            return dir;
        }
        let n = r.u32().unwrap_or(0);
        let mut entries = Vec::new();
        for _ in 0..n {
            let (Some(name), Some(off), Some(len)) = (r.str(), r.u32(), r.u32()) else {
                break;
            };
            entries.push((name, off as usize, len as usize));
        }
        let base = r.p;
        for (name, off, len) in entries {
            dir.insert(name, (base + off, len));
        }
        dir
    })
}

fn inflate(name: &str) -> Option<Vec<u8>> {
    let &(off, len) = directory().get(name)?;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(DATA.get(off..off + len)?)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(b)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u8()? as usize;
        std::str::from_utf8(self.take(n)?).ok()
    }
}

/// A predefined CMap by name (`None`: not one of Adobe's).
pub fn cmap(name: &str) -> Option<Arc<Predefined>> {
    load(name, 0)
}

fn load(name: &str, depth: u32) -> Option<Arc<Predefined>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Predefined>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
        return v.clone();
    }
    let v = (depth < MAX_DEPTH)
        .then(|| parse(name, depth))
        .flatten()
        .map(Arc::new);
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_owned(), v.clone());
    v
}

fn parse(name: &str, depth: u32) -> Option<Predefined> {
    let data = inflate(name)?;
    let mut r = Reader { d: &data, p: 0 };
    if r.u8()? != 0 {
        return None;
    }
    let vertical = r.u8()? == 1;
    let ordering = r.str()?.to_owned();
    let uses = r.str()?.to_owned();
    let parent = (!uses.is_empty()).then(|| load(&uses, depth + 1)).flatten();
    let mut codespace = Vec::new();
    for _ in 0..r.u32()? {
        codespace.push((r.u8()?, r.u32()?, r.u32()?));
    }
    if let Some(p) = &parent {
        for c in &p.codespace {
            if !codespace.contains(c) {
                codespace.push(*c);
            }
        }
    }
    let n = r.u32()? as usize;
    let mut ranges = Vec::with_capacity(n);
    for _ in 0..n {
        ranges.push((r.u8()?, r.u32()?, r.u32()?, r.u32()?));
    }
    Some(Predefined {
        name: name.to_owned(),
        vertical,
        ordering,
        codespace,
        ranges,
        parent,
    })
}

/// CID → Unicode of a character collection by its ordering (`Japan1`).
pub fn cid_text(ordering: &str) -> Option<Arc<CidText>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<CidText>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = g.get(ordering) {
        return v.clone();
    }
    let v = parse_text(&format!("Adobe-{ordering}-UCS2")).map(Arc::new);
    g.insert(ordering.to_owned(), v.clone());
    v
}

fn parse_text(name: &str) -> Option<CidText> {
    let data = inflate(name)?;
    let mut r = Reader { d: &data, p: 0 };
    if r.u8()? != 1 {
        return None;
    }
    let n = r.u32()? as usize;
    let mut ranges = Vec::with_capacity(n);
    for _ in 0..n {
        let (lo, hi, k) = (r.u32()?, r.u32()?, r.u8()?);
        let u = (0..k).map(|_| r.u32()).collect::<Option<Vec<u32>>>()?;
        ranges.push((lo, hi, u));
    }
    Some(CidText { ranges })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_jis_and_unicode() {
        let h = cmap("90ms-RKSJ-H").unwrap();
        assert_eq!(h.ordering, "Japan1");
        assert!(!h.vertical);
        // あ (0x82a0), 漢 (0x8abf), half-width A, half-width katakana ｱ.
        assert_eq!(h.cid((2, 0x82a0)), Some(843));
        assert_eq!(h.cid((2, 0x8abf)), Some(1533));
        assert_eq!(h.cid((1, 0x41)), Some(264));
        assert_eq!(h.cid((1, 0xb1)), Some(343));
        assert!(h.codespace.contains(&(2, 0x8140, 0x9ffc)));
        // The vertical CMap: its own vertical forms (、 is 0x8141), the
        // rest through usecmap.
        let v = cmap("90ms-RKSJ-V").unwrap();
        assert!(v.vertical);
        assert_eq!(v.cid((2, 0x8141)), Some(7887));
        assert_eq!(v.cid((2, 0x82a0)), Some(843));
        assert_eq!(v.codespace, h.codespace);
        let u = cmap("UniJIS-UCS2-H").unwrap();
        assert_eq!(u.cid((2, 0x3042)), Some(843));
        assert!(cmap("No-Such-CMap").is_none());

        let t = cid_text("Japan1").unwrap();
        assert_eq!(t.text(843).as_deref(), Some("あ"));
        assert_eq!(t.text(1533).as_deref(), Some("漢"));
        assert_eq!(t.text(34).as_deref(), Some("A"));
        assert!(cid_text("GB1").is_some());
    }
}
