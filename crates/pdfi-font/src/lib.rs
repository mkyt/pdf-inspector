//! Fonts: how strings split into character codes, the width of each code,
//! the text it stands for, and the glyph that draws it.
//!
//! [`load`] reads a font dictionary (and its descendant, descriptor,
//! `/Encoding` and `/ToUnicode` CMaps); that is all the content
//! interpreter needs for advances, bounding boxes and decoded text. The
//! font program is read only when a glyph is first asked for
//! ([`Font::outline`]): the embedded one ([`program`]), or for simple
//! fonts that are not embedded a substitute ([`substitute`]). Type 3 glyphs
//! are content streams ([`Font::char_proc`]) that the interpreter runs.
//!
//! Where widths have to be guessed (standard 14 fonts outside ASCII,
//! predefined CJK CMaps) the font is marked `approximate`.

pub mod cmap;
pub mod encoding;
pub mod predefined;
pub mod program;
mod std14;
pub mod substitute;
mod tables;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pdfi_model::resolve;
use pdfi_syntax::{Dict, DictExt, Name, ObjId, Object, PdfFile, Severity, Stream};

use crate::cmap::CodeKey;
use crate::encoding::{Table, glyph_text};
pub use crate::program::{Format, Outline, Program, Seg};
use crate::substitute::Traits;

/// Largest font program read.
const MAX_PROGRAM: usize = 32 << 20;
/// Outlines kept per font.
const MAX_OUTLINES: usize = 1 << 14;
/// Largest `/CIDToGIDMap` read (two bytes per CID).
const MAX_CID_MAP: usize = 2 << 16;

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

/// `/CIDToGIDMap` of a CIDFontType2 font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CidToGid {
    Identity,
    Map(Vec<u16>),
}

/// The stream that holds an embedded font program.
#[derive(Clone, Debug)]
pub struct ProgramRef {
    pub id: Option<ObjId>,
    /// `FontFile`, `FontFile2` or `FontFile3`.
    pub key: &'static str,
    /// `/Subtype` of a `FontFile3` stream.
    pub subtype: Option<String>,
    stream: Stream,
}

/// What draws a font's glyphs.
#[derive(Clone, Debug)]
pub enum GlyphSource {
    Embedded(Arc<Program>),
    /// A simple font that is not embedded (or whose program cannot be
    /// read), drawn with a compiled-in font.
    Substitute {
        name: &'static str,
        program: Arc<Program>,
    },
    /// Type 3: glyph procedures.
    Procedures,
    /// Nothing: glyphs are drawn as boxes.
    None,
}

/// The font program, read on first use.
#[derive(Debug)]
pub struct Glyphs {
    pub source: GlyphSource,
    /// Problems reading or choosing it.
    pub issues: Vec<FontIssue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontIssue {
    pub severity: Severity,
    pub message: String,
}

impl FontIssue {
    fn new(severity: Severity, message: impl Into<String>) -> Self {
        FontIssue {
            severity,
            message: message.into(),
        }
    }
}

/// The glyph a code selects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphRef {
    pub cid: Option<u32>,
    pub gid: Option<u16>,
    pub name: Option<String>,
}

/// How to get an outline.
enum Pick {
    Gid(u16),
    Name(String),
    None,
}

/// Type 3 font data.
#[derive(Clone, Debug)]
pub struct Type3 {
    /// Glyph space to text space.
    pub matrix: [f64; 6],
    procs: Option<Dict>,
    /// The font's `/Resources` (the glyph procedures use the page's
    /// without it).
    pub resources: Option<Object>,
}

pub struct Font {
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
    /// `/FontDescriptor /Flags`.
    pub flags: Option<i64>,
    /// Symbolic (flag, or the Symbol / ZapfDingbats fonts).
    pub symbolic: bool,
    pub descriptor: Option<ObjId>,
    /// Type 0: the descendant font and its `/Subtype`.
    pub descendant: Option<ObjId>,
    pub cid_subtype: Option<String>,
    /// Type 0: `/CIDSystemInfo /Ordering` of the descendant font.
    pub ordering: Option<String>,
    /// How the encoding was given: `WinAnsiEncoding`, `Identity-H`,
    /// `embedded CMap`, ...
    pub encoding: Option<String>,
    /// Entries of `/Differences`.
    pub differences: u32,
    pub to_unicode: Option<ObjId>,
    /// Entries of the `/ToUnicode` CMap.
    pub to_unicode_entries: usize,
    pub cid_to_gid: Option<CidToGid>,
    pub program: Option<ProgramRef>,
    pub type3: Option<Type3>,
    /// `/FirstChar`, and the number of `/Widths`.
    pub first_char: u32,
    pub widths_given: usize,
    /// Problems of the dictionaries.
    pub issues: Vec<FontIssue>,
    codes: Codes,
    /// Simple fonts: widths by code (glyph space units).
    widths: Vec<f64>,
    missing_width: f64,
    /// Type 0: code → CID (`None`: identity), from an embedded CMap.
    cids: Option<HashMap<CodeKey, u32>>,
    /// Type 0: a predefined CMap (by name, or used by an embedded one),
    /// looked up after `cids`.
    predefined: Option<Arc<predefined::Predefined>>,
    cid_widths: CidWidths,
    unicode: HashMap<CodeKey, String>,
    /// Simple fonts: glyph names from `/Differences` (256 or empty).
    diff_names: Vec<Option<Box<str>>>,
    /// The base encoding the dictionary names.
    base_names: Option<&'static Table>,
    /// The encoding text is read with when the dictionary names none.
    default_names: Option<&'static Table>,
    utf16_codes: bool,
    traits: (f64, Option<f64>),
    glyphs: OnceLock<Glyphs>,
    outlines: Mutex<HashMap<CodeKey, Option<Arc<Outline>>>>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font")
            .field("id", &self.id)
            .field("base_font", &self.base_font)
            .field("subtype", &self.subtype)
            .finish_non_exhaustive()
    }
}

/// One character code of a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub code: u32,
    pub len: u8,
}

impl Code {
    pub fn key(self) -> CodeKey {
        (self.len, self.code)
    }

    /// The bytes as hex (`0041`).
    pub fn hex(self) -> String {
        let n = self.len.clamp(1, 4) as usize;
        format!("{:0w$X}", self.code, w = n * 2)
    }
}

impl Font {
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

    /// The CID of a code (Type 0 fonts).
    pub fn cid(&self, c: Code) -> u32 {
        if let Some(&cid) = self.cids.as_ref().and_then(|m| m.get(&c.key())) {
            return cid;
        }
        if let Some(p) = &self.predefined {
            return p.cid(c.key()).unwrap_or(0);
        }
        match self.cids {
            Some(_) => 0,
            None => c.code,
        }
    }

    /// The character collection of a Type 0 font: `/CIDSystemInfo`, else
    /// that of its predefined CMap.
    fn collection(&self) -> Option<&str> {
        self.ordering
            .as_deref()
            .filter(|o| *o != "Identity")
            .or(self.predefined.as_ref().map(|p| p.ordering.as_str()))
    }

    /// Horizontal (or, for vertical fonts, vertical) advance of `c` in text
    /// space per unit of font size. Vertical advances are negative (down).
    pub fn advance(&self, c: Code) -> f64 {
        match self.kind {
            FontKind::Type0 => {
                if self.vertical {
                    // /W2 is not read: the default vertical metrics.
                    -1.0
                } else {
                    self.cid_widths.get(self.cid(c)) * self.scale
                }
            }
            _ => self.width(c.code) * self.scale,
        }
    }

    /// Width of a simple font's code in glyph space.
    fn width(&self, code: u32) -> f64 {
        code.checked_sub(self.first_char)
            .and_then(|i| self.widths.get(i as usize).copied())
            .unwrap_or(self.missing_width)
    }

    /// Width in glyph space (thousandths of an em for all but Type 3).
    pub fn glyph_width(&self, c: Code) -> f64 {
        match self.kind {
            FontKind::Type0 => self.cid_widths.get(self.cid(c)),
            _ => self.width(c.code),
        }
    }

    /// The glyph name `/Encoding` (with `/Differences`) gives a code of a
    /// simple font.
    pub fn encoded_name(&self, code: u32) -> Option<&str> {
        let i = usize::try_from(code).ok().filter(|&i| i < 256)?;
        if let Some(Some(n)) = self.diff_names.get(i) {
            return Some(n);
        }
        self.base_names.map(|t| t[i]).filter(|n| !n.is_empty())
    }

    /// The name text is read with: the encoding's, or the default one.
    fn text_name(&self, code: u32) -> Option<&str> {
        self.encoded_name(code).or_else(|| {
            let t = self.default_names?;
            t.get(code as usize).copied().filter(|n| !n.is_empty())
        })
    }

    /// The text `c` stands for, when it can be told.
    pub fn text(&self, c: Code) -> Option<String> {
        if let Some(t) = self.unicode.get(&c.key()) {
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
        if self.kind == FontKind::Type0 {
            // The CID's text in its character collection.
            let t = predefined::cid_text(self.collection()?)?;
            return t.text(self.cid(c));
        }
        if c.len != 1 {
            return None;
        }
        self.text_name(c.code).and_then(glyph_text)
    }

    /// A Type 0 font whose codes give Adobe-Japan1 CIDs.
    fn japan1(&self) -> bool {
        let mapped = match &self.cids {
            None => true,
            Some(m) => !m.is_empty() || self.predefined.is_some(),
        };
        mapped && self.collection() == Some("Japan1")
    }

    /// Text is known for the codes (a `/ToUnicode` CMap, UCS-2 codes, or
    /// the CIDs of a known character collection).
    fn unicode_known(&self) -> bool {
        !self.unicode.is_empty()
            || self.utf16_codes
            || self
                .collection()
                .is_some_and(|o| predefined::cid_text(o).is_some())
    }

    /// Whether word spacing applies: single-byte code 32.
    pub fn is_space(&self, c: Code) -> bool {
        c.len == 1 && c.code == 32
    }

    /// The font program (read on first use).
    pub fn glyphs(&self, file: &PdfFile) -> &Glyphs {
        self.glyphs.get_or_init(|| self.load_glyphs(file))
    }

    fn load_glyphs(&self, file: &PdfFile) -> Glyphs {
        let mut issues = Vec::new();
        if self.kind == FontKind::Type3 {
            return Glyphs {
                source: GlyphSource::Procedures,
                issues,
            };
        }
        if let Some(p) = &self.program {
            let d = file.decode_stream(
                p.id.unwrap_or(ObjId::new(0, 0)),
                &p.stream,
                Some(MAX_PROGRAM),
            );
            let what = match &p.subtype {
                Some(s) => format!("/{} /{s}", p.key),
                None => format!("/{}", p.key),
            };
            if let Some(e) = d.errors.first() {
                issues.push(FontIssue::new(Severity::Error, format!("{what}: {e}")));
            }
            match Program::parse(Cow::Owned(d.data)) {
                Ok(prog) => {
                    let expected = match (p.key, p.subtype.as_deref()) {
                        ("FontFile", _) => Some(Format::Type1),
                        ("FontFile2", _) => Some(Format::TrueType),
                        ("FontFile3", Some("Type1C" | "CIDFontType0C")) => Some(Format::Cff),
                        ("FontFile3", Some("OpenType")) => None,
                        _ => None,
                    };
                    if expected.is_some_and(|e| e != prog.format) {
                        issues.push(FontIssue::new(
                            Severity::Info,
                            format!("{what} holds a {} program", prog.format.name()),
                        ));
                    }
                    return Glyphs {
                        source: GlyphSource::Embedded(Arc::new(prog)),
                        issues,
                    };
                }
                Err(e) => issues.push(FontIssue::new(
                    Severity::Error,
                    format!("{what} cannot be read: {e}"),
                )),
            }
        }
        let traits = Traits {
            base_font: self.base_font.as_deref(),
            flags: self.flags.unwrap_or(0),
            weight: self.traits.1,
            italic_angle: self.traits.0,
        };
        let sub = if self.kind == FontKind::Type0 {
            // Japanese glyphs: by CID for Adobe-Japan1, else by the text.
            let usable = self.japan1() || self.unicode_known();
            match usable
                .then(|| substitute::substitute_cjk(&traits))
                .flatten()
            {
                Some(s) => Some(s),
                None => {
                    issues.push(FontIssue::new(
                        Severity::Warning,
                        if self.program.is_none() {
                            "the CID font is not embedded and there is no substitute for it: glyphs are drawn as boxes"
                        } else {
                            "there is no substitute for the CID font: glyphs are drawn as boxes"
                        },
                    ));
                    return Glyphs {
                        source: GlyphSource::None,
                        issues,
                    };
                }
            }
        } else {
            substitute::substitute(&traits)
        };
        match sub {
            Some((name, program)) => {
                issues.push(FontIssue::new(
                    Severity::Info,
                    if self.program.is_some() {
                        format!("drawn with {name} instead")
                    } else {
                        format!("not embedded: drawn with {name}")
                    },
                ));
                Glyphs {
                    source: GlyphSource::Substitute { name, program },
                    issues,
                }
            }
            None => Glyphs {
                source: GlyphSource::None,
                issues,
            },
        }
    }

    /// Choose the glyph of a code.
    fn pick(&self, g: &Glyphs, c: Code) -> (GlyphRef, Pick) {
        let mut r = GlyphRef::default();
        match (&g.source, self.kind) {
            (GlyphSource::Procedures | GlyphSource::None, _) | (_, FontKind::Type3) => {
                if self.kind != FontKind::Type0 {
                    r.name = self.encoded_name(c.code).map(String::from);
                } else {
                    r.cid = Some(self.cid(c));
                }
                (r, Pick::None)
            }
            (GlyphSource::Embedded(p), FontKind::Type0) => {
                let cid = self.cid(c);
                r.cid = Some(cid);
                let gid = match p.format {
                    Format::TrueType => match &self.cid_to_gid {
                        Some(CidToGid::Map(m)) => m.get(cid as usize).copied(),
                        _ => u16::try_from(cid).ok(),
                    },
                    Format::OpenType | Format::Cff => p.gid_of_cid(cid),
                    Format::Type1 => None,
                };
                r.gid = gid;
                (r, gid.map_or(Pick::None, Pick::Gid))
            }
            (GlyphSource::Embedded(p), _) => self.pick_simple(p, c, r),
            (GlyphSource::Substitute { program: p, .. }, FontKind::Type0) => {
                let cid = self.cid(c);
                r.cid = Some(cid);
                let gid = if self.japan1() {
                    p.gid_of_cid(cid)
                } else {
                    self.text(c)
                        .and_then(|t| t.chars().next())
                        .and_then(|u| p.cmap(3, 10, u as u32).or_else(|| p.cmap(3, 1, u as u32)))
                };
                // Vertical writing: the substitute's vertical forms (the
                // CIDs of Identity-V and of horizontal-form text are
                // horizontal ones; vertical forms map to themselves).
                let gid = gid.map(|g| if self.vertical { p.vertical_gid(g) } else { g });
                r.gid = gid;
                (r, gid.map_or(Pick::None, Pick::Gid))
            }
            (GlyphSource::Substitute { .. }, _) => {
                // The encoding's names, or the built-in encoding of the
                // standard font it stands for.
                let name = self.text_name(c.code).map(String::from);
                r.name = name.clone();
                (r, name.map_or(Pick::None, Pick::Name))
            }
        }
    }

    /// The glyph of a simple font's code in its embedded program.
    fn pick_simple(&self, p: &Program, c: Code, mut r: GlyphRef) -> (GlyphRef, Pick) {
        let code = c.code as u8;
        let named = self.encoded_name(c.code).map(String::from);
        match p.format {
            Format::Type1 => {
                let name = named.or_else(|| p.builtin_name(code));
                r.name = name.clone();
                (r, name.map_or(Pick::None, Pick::Name))
            }
            Format::Cff if p.cid_keyed => {
                r.gid = Some(code.into());
                (r, Pick::Gid(code.into()))
            }
            Format::Cff => {
                let gid = named
                    .as_deref()
                    .and_then(|n| p.gid_by_name(n))
                    .or_else(|| p.builtin_gid(code));
                r.name = named;
                r.gid = gid;
                (r, gid.map_or(Pick::None, Pick::Gid))
            }
            Format::TrueType | Format::OpenType => {
                // The rules of ISO 32000 9.6.5.4, then the glyph names, then
                // the code itself (subsets without a cmap).
                let mut gid = None;
                let use_names = named.is_some() && !(self.symbolic && self.diff_names.is_empty());
                if use_names {
                    let n = named.as_deref().unwrap_or_default();
                    if let Some(u) = glyph_text(n).and_then(|t| t.chars().next()) {
                        gid = p.cmap(3, 1, u as u32).or_else(|| p.cmap(0, 3, u as u32));
                    }
                }
                if gid.is_none() {
                    let c = u32::from(code);
                    gid = [c, 0xf000 | c, 0xf100 | c, 0xf200 | c]
                        .into_iter()
                        .find_map(|v| p.cmap(3, 0, v));
                }
                if gid.is_none() {
                    let mac = if use_names {
                        named.as_deref().and_then(encoding::mac_roman_code)
                    } else {
                        Some(code)
                    };
                    gid = mac.and_then(|m| p.cmap(1, 0, m.into()));
                }
                if gid.is_none() {
                    gid = named.as_deref().and_then(|n| p.gid_by_name(n));
                }
                if gid.is_none() && !use_names {
                    gid = p.cmap(3, 1, code.into());
                }
                if gid.is_none() && !p.has_any_cmap() {
                    gid = Some(code.into())
                        .filter(|&g: &u16| p.glyphs.is_none_or(|n| u32::from(g) < n));
                }
                r.name = named;
                r.gid = gid;
                (r, gid.map_or(Pick::None, Pick::Gid))
            }
        }
    }

    /// The glyph a code selects (for listing glyphs).
    pub fn glyph(&self, file: &PdfFile, c: Code) -> GlyphRef {
        let g = self.glyphs(file);
        let (mut r, _) = self.pick(g, c);
        if r.name.is_none() {
            if let (Some(gid), GlyphSource::Embedded(p)) = (r.gid, &g.source) {
                r.name = p.glyph_name(gid);
            }
        }
        r
    }

    /// The outline that draws a code (`None`: no glyph, or the font has
    /// none; see [`Font::glyphs`]). Cached.
    pub fn outline(&self, file: &PdfFile, c: Code) -> Option<Arc<Outline>> {
        let key = c.key();
        if let Some(o) = lock(&self.outlines).get(&key) {
            return o.clone();
        }
        let g = self.glyphs(file);
        let (_, pick) = self.pick(g, c);
        let program = match &g.source {
            GlyphSource::Embedded(p) | GlyphSource::Substitute { program: p, .. } => Some(p),
            _ => None,
        };
        let o = program
            .and_then(|p| match pick {
                Pick::Gid(gid) => p.outline(gid),
                Pick::Name(n) => p.outline_by_name(&n),
                Pick::None => None,
            })
            .map(Arc::new);
        let mut m = lock(&self.outlines);
        if m.len() >= MAX_OUTLINES {
            m.clear();
        }
        m.insert(key, o.clone());
        o
    }

    /// Whether glyphs have outlines (embedded or substituted program).
    pub fn has_outlines(&self, file: &PdfFile) -> bool {
        matches!(
            self.glyphs(file).source,
            GlyphSource::Embedded(_) | GlyphSource::Substitute { .. }
        )
    }

    /// The glyph procedure of a Type 3 font's code.
    pub fn char_proc(&self, file: &PdfFile, c: Code) -> Option<(Option<ObjId>, Arc<Object>)> {
        let t = self.type3.as_ref()?;
        let name = self.encoded_name(c.code)?;
        let o = t.procs.as_ref()?.get(name.as_bytes())?;
        let (id, r) = resolve(file, &o.obj);
        r.as_stream()?;
        Some((id, Arc::new((*r).clone())))
    }

    /// Codes a listing of the glyphs shows: all single bytes for simple
    /// fonts; for Type 0 the codes the CMaps name, or the CIDs of an
    /// identity encoding up to what `/CIDToGIDMap` maps, the glyph count, or
    /// the largest CID with a width.
    pub fn listed_codes(&self, file: &PdfFile) -> Vec<Code> {
        if self.kind != FontKind::Type0 {
            return (0..256).map(|code| Code { code, len: 1 }).collect();
        }
        let mut keys: Vec<CodeKey> = self.unicode.keys().copied().collect();
        match &self.cids {
            Some(m) if !m.is_empty() => keys.extend(m.keys().copied()),
            Some(_) => {}
            None => {
                let n = match (&self.cid_to_gid, &self.glyphs(file).source) {
                    (Some(CidToGid::Map(m)), _) => Some(m.len() as u32),
                    (_, GlyphSource::Embedded(p)) => p.glyphs,
                    _ => None,
                }
                .unwrap_or_else(|| {
                    self.cid_widths
                        .ranges
                        .last()
                        .map_or(0, |r| r.1.saturating_add(1))
                })
                .min(1 << 16);
                let len = match &self.codes {
                    Codes::Ranges(r) => r.first().map_or(2, |x| x.0),
                    _ => 2,
                };
                keys.extend((0..n).map(|c| (len, c)));
            }
        }
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter()
            .take(1 << 16)
            .map(|(len, code)| Code { code, len })
            .collect()
    }

    /// A font object that is not a font dictionary.
    pub fn unknown() -> Self {
        Font {
            id: None,
            base_font: None,
            subtype: None,
            kind: FontKind::Simple,
            vertical: false,
            scale: 0.001,
            ascent: 0.8,
            descent: -0.2,
            approximate: true,
            flags: None,
            symbolic: false,
            descriptor: None,
            descendant: None,
            cid_subtype: None,
            ordering: None,
            encoding: None,
            differences: 0,
            to_unicode: None,
            to_unicode_entries: 0,
            cid_to_gid: None,
            program: None,
            type3: None,
            first_char: 0,
            widths_given: 0,
            issues: Vec::new(),
            codes: Codes::OneByte,
            widths: Vec::new(),
            missing_width: 500.0,
            cids: None,
            predefined: None,
            cid_widths: CidWidths::default(),
            unicode: HashMap::new(),
            diff_names: Vec::new(),
            base_names: None,
            default_names: None,
            utf16_codes: false,
            traits: (0.0, None),
            glyphs: OnceLock::new(),
            outlines: Mutex::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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
pub fn load(file: &PdfFile, id: Option<ObjId>, d: &Dict) -> Font {
    let mut f = Font::unknown();
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
        let (uid, r) = resolve(file, u);
        if let Some(s) = r.as_stream() {
            let cm = cmap::read(file, u, s);
            if let Some(p) = cm.problem {
                f.issues.push(FontIssue::new(
                    Severity::Warning,
                    format!("/ToUnicode: {p}"),
                ));
            }
            f.to_unicode = uid;
            f.to_unicode_entries = cm.unicode.len();
            f.unicode = cm.unicode;
        } else if !matches!(&*r, Object::Name(_)) {
            f.issues.push(FontIssue::new(
                Severity::Warning,
                format!("/ToUnicode is not a stream ({})", r.type_name()),
            ));
        }
    }
    match f.kind {
        FontKind::Type0 => type0(file, d, &mut f),
        FontKind::Simple | FontKind::Type3 => simple(file, d, &mut f),
    }
    f
}

/// Read the descriptor: metrics, flags and the embedded program.
fn descriptor(file: &PdfFile, d: &Dict, f: &mut Font) -> Option<()> {
    let o = d.get_key("FontDescriptor")?;
    let (id, fd) = resolve(file, o);
    let Some(fd) = fd.as_dict() else {
        f.issues.push(FontIssue::new(
            Severity::Error,
            format!("/FontDescriptor is not a dictionary ({})", fd.type_name()),
        ));
        return None;
    };
    f.descriptor = id;
    let get = |k: &str| fd.get_key(k).and_then(|o| num(file, o));
    f.flags = fd
        .get_key("Flags")
        .and_then(|o| resolve(file, o).1.as_i64());
    f.symbolic = f.flags.is_some_and(|fl| fl & 4 != 0 && fl & 32 == 0);
    f.missing_width = get("MissingWidth").unwrap_or(f.missing_width);
    f.traits = (get("ItalicAngle").unwrap_or(0.0), get("FontWeight"));
    if f.kind != FontKind::Type3 {
        let bbox = fd
            .get_key("FontBBox")
            .map(|o| numbers(file, o))
            .unwrap_or_default();
        let (asc, desc) = (get("Ascent").unwrap_or(0.0), get("Descent").unwrap_or(0.0));
        if asc != 0.0 || desc != 0.0 {
            f.ascent = asc * 0.001;
            f.descent = desc * 0.001;
        } else if bbox.len() == 4 && bbox[3] > bbox[1] {
            f.ascent = bbox[3] * 0.001;
            f.descent = bbox[1] * 0.001;
        }
    }
    for key in ["FontFile", "FontFile2", "FontFile3"] {
        let Some(o) = fd.get_key(key) else { continue };
        let (pid, r) = resolve(file, o);
        match r.as_stream() {
            Some(s) => {
                if f.program.is_some() {
                    f.issues.push(FontIssue::new(
                        Severity::Warning,
                        format!("more than one font program; /{key} is ignored"),
                    ));
                    continue;
                }
                f.program = Some(ProgramRef {
                    id: pid,
                    key,
                    subtype: s
                        .dict
                        .get_name("Subtype")
                        .map(|n| n.as_str_lossy().into_owned()),
                    stream: s.clone(),
                });
            }
            None => f.issues.push(FontIssue::new(
                Severity::Error,
                format!("/{key} is not a stream ({})", r.type_name()),
            )),
        }
    }
    Some(())
}

fn simple(file: &PdfFile, d: &Dict, f: &mut Font) {
    f.first_char = d
        .get_key("FirstChar")
        .and_then(|o| resolve(file, o).1.as_i64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    f.widths = d
        .get_key("Widths")
        .map(|o| numbers(file, o))
        .unwrap_or_default();
    f.widths_given = f.widths.len();
    if let Some(last) = d
        .get_key("LastChar")
        .and_then(|o| resolve(file, o).1.as_i64())
    {
        let want = last - i64::from(f.first_char) + 1;
        if !f.widths.is_empty() && want != f.widths.len() as i64 {
            f.issues.push(FontIssue::new(
                Severity::Warning,
                format!(
                    "/Widths has {} entries; /FirstChar {} … /LastChar {last} needs {want}",
                    f.widths.len(),
                    f.first_char
                ),
            ));
        }
    }
    descriptor(file, d, f);
    let std = f.base_font.as_deref().and_then(encoding::std14);
    let std_symbolic = std.is_some_and(|s| s.symbolic);
    f.symbolic |= std_symbolic;
    if f.kind == FontKind::Type3 {
        let m = d
            .get_key("FontMatrix")
            .map(|o| numbers(file, o))
            .unwrap_or_default();
        let m: [f64; 6] = match <[f64; 6]>::try_from(m) {
            Ok(m) => m,
            Err(_) => {
                f.issues.push(FontIssue::new(
                    Severity::Error,
                    "Type 3 font without a valid /FontMatrix",
                ));
                [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]
            }
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
        let procs = d
            .get_key("CharProcs")
            .and_then(|o| resolve(file, o).1.as_dict().cloned());
        if procs.is_none() {
            f.issues.push(FontIssue::new(
                Severity::Error,
                "Type 3 font without /CharProcs",
            ));
        }
        f.type3 = Some(Type3 {
            matrix: m,
            procs,
            resources: d
                .get_key("Resources")
                .map(|o| (*resolve(file, o).1).clone()),
        });
    }
    // Encoding: glyph names by code.
    let enc = d.get_key("Encoding").map(|o| resolve(file, o).1);
    let mut differences = None;
    let mut desc = None;
    match enc.as_deref() {
        Some(Object::Name(n)) => {
            f.base_names = encoding::base(n.as_bytes());
            desc = Some(n.as_str_lossy().into_owned());
            if f.base_names.is_none() {
                f.issues.push(FontIssue::new(
                    Severity::Warning,
                    format!("unknown /Encoding /{}", n.as_str_lossy()),
                ));
            }
        }
        Some(o) => {
            if let Some(ed) = o.as_dict() {
                let base = name_of(file, ed, "BaseEncoding");
                f.base_names = base.as_ref().and_then(|n| encoding::base(n.as_bytes()));
                desc = base.map(|n| n.as_str_lossy().into_owned());
                differences = ed.get_key("Differences").cloned();
            } else {
                f.issues.push(FontIssue::new(
                    Severity::Warning,
                    format!("/Encoding is not a name or dictionary ({})", o.type_name()),
                ));
            }
        }
        None => {}
    }
    f.default_names = Some(match (std, f.subtype.as_deref()) {
        (Some(s), _) if s.symbolic => s.encoding,
        (_, Some("TrueType")) => encoding::win_ansi(),
        _ => encoding::standard(),
    });
    if let Some(diff) = differences {
        let mut names: Vec<Option<Box<str>>> = vec![None; 256];
        if let Some(a) = resolve(file, &diff).1.as_array() {
            let mut code = 0usize;
            for n in a {
                match &n.obj {
                    Object::Integer(i) => code = (*i).clamp(0, 255) as usize,
                    Object::Name(g) => {
                        if code < 256 {
                            names[code] = Some(g.as_str_lossy().into());
                            f.differences += 1;
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        f.diff_names = names;
        let base = desc.take().unwrap_or_else(|| "built-in".into());
        desc = Some(format!("{base} + {} differences", f.differences));
    }
    f.encoding = desc;
    if f.widths.is_empty() {
        match std {
            Some(s) => {
                // The AFM widths of the glyphs the codes name.
                f.first_char = 0;
                f.widths = (0..256)
                    .map(|c| match f.text_name(c) {
                        Some(n) => s.width(n).unwrap_or(500.0),
                        None => 0.0,
                    })
                    .collect();
            }
            None => {
                f.missing_width = 500.0;
                f.approximate = true;
                if f.kind != FontKind::Type3 {
                    f.issues.push(FontIssue::new(
                        Severity::Warning,
                        "no /Widths and not a standard 14 font: widths are guessed",
                    ));
                }
            }
        }
    }
}

fn type0(file: &PdfFile, d: &Dict, f: &mut Font) {
    f.codes = Codes::TwoBytes;
    let enc = d.get_key("Encoding");
    match enc.map(|o| resolve(file, o)) {
        Some((_, r)) if matches!(&*r, Object::Name(_)) => {
            let n = r.as_name().cloned().unwrap_or_else(|| Name::from(""));
            let s = n.as_bytes();
            f.vertical = s.ends_with(b"-V");
            f.encoding = Some(n.as_str_lossy().into_owned());
            if s != b"Identity-H" && s != b"Identity-V" {
                let name = n.as_str_lossy();
                f.utf16_codes = name.contains("UCS2") || name.contains("UTF16");
                // Codes are not CIDs: the predefined CMap maps them.
                f.cids = Some(HashMap::new());
                match predefined::cmap(&name) {
                    Some(p) => {
                        f.codes = Codes::Ranges(p.codespace.clone());
                        f.vertical = p.vertical;
                        f.predefined = Some(p);
                    }
                    None => {
                        f.codes = predefined_codes(&name);
                        f.approximate = true;
                        f.issues.push(FontIssue::new(
                            Severity::Warning,
                            format!(
                                "/Encoding /{name} is not a predefined CMap: CIDs are not known"
                            ),
                        ));
                    }
                }
            }
        }
        Some((cid, r)) => match r.as_stream() {
            Some(s) => {
                let cm = cmap::read(file, enc.unwrap_or(&Object::Null), s);
                f.vertical = cm.vertical
                    || s.dict
                        .get_key("WMode")
                        .and_then(|o| resolve(file, o).1.as_i64())
                        == Some(1);
                // A predefined CMap it builds on (`usecmap`, `/UseCMap`).
                let uses = cm.uses.clone().or_else(|| {
                    let o = s.dict.get_key("UseCMap")?;
                    let (_, o) = resolve(file, o);
                    o.as_name().map(|n| n.as_str_lossy().into_owned())
                });
                if let Some(u) = uses {
                    match predefined::cmap(&u) {
                        Some(p) => f.predefined = Some(p),
                        None => f.issues.push(FontIssue::new(
                            Severity::Info,
                            format!("the encoding CMap uses /{u}, which is not read"),
                        )),
                    }
                }
                let mut space = cm.codespace;
                if let Some(p) = &f.predefined {
                    for c in &p.codespace {
                        if !space.contains(c) {
                            space.push(*c);
                        }
                    }
                    f.vertical |= p.vertical;
                }
                if !space.is_empty() {
                    f.codes = Codes::Ranges(space);
                }
                if let Some(p) = cm.problem {
                    f.issues
                        .push(FontIssue::new(Severity::Warning, format!("/Encoding: {p}")));
                }
                f.encoding = Some(match cid {
                    Some(id) => format!("embedded CMap ({} {} R)", id.num, id.generation),
                    None => "embedded CMap".into(),
                });
                f.cids = Some(cm.cids);
            }
            None => f.issues.push(FontIssue::new(
                Severity::Error,
                format!("/Encoding is not a name or CMap stream ({})", r.type_name()),
            )),
        },
        None => f.issues.push(FontIssue::new(
            Severity::Error,
            "Type 0 font without /Encoding",
        )),
    }
    let desc = d
        .get_key("DescendantFonts")
        .map(|o| resolve(file, o).1)
        .and_then(|a| a.as_array().and_then(|a| a.first().map(|n| n.obj.clone())));
    let Some(desc) = desc else {
        f.issues.push(FontIssue::new(
            Severity::Error,
            "Type 0 font without /DescendantFonts",
        ));
        f.approximate = true;
        f.cid_widths.default = 1000.0;
        return;
    };
    let (did, dr) = resolve(file, &desc);
    let Some(dd) = dr.as_dict() else {
        f.approximate = true;
        f.cid_widths.default = 1000.0;
        return;
    };
    f.descendant = did;
    f.cid_subtype = name_of(file, dd, "Subtype").map(|n| n.as_str_lossy().into_owned());
    f.ordering = dd
        .get_key("CIDSystemInfo")
        .map(|o| resolve(file, o).1)
        .and_then(|i| {
            let o = i.as_dict()?.get_key("Ordering")?;
            let (_, o) = resolve(file, o);
            o.as_string()
                .map(|s| String::from_utf8_lossy(&s.bytes).into_owned())
        });
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
                if f.cid_widths.ranges.len() > cmap::MAX_ENTRIES {
                    break;
                }
            }
        }
    }
    f.cid_widths.ranges.sort_by_key(|r| r.0);
    if let Some(m) = dd.get_key("CIDToGIDMap") {
        let (mid, r) = resolve(file, m);
        f.cid_to_gid = match &*r {
            Object::Name(n) if n == "Identity" => Some(CidToGid::Identity),
            Object::Stream(s) => {
                let data = file
                    .decode_stream(mid.unwrap_or(ObjId::new(0, 0)), s, Some(MAX_CID_MAP))
                    .data;
                Some(CidToGid::Map(
                    data.chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect(),
                ))
            }
            other => {
                f.issues.push(FontIssue::new(
                    Severity::Warning,
                    format!(
                        "/CIDToGIDMap is not /Identity or a stream ({})",
                        other.type_name()
                    ),
                ));
                None
            }
        };
    }
    descriptor(file, dd, f);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_ranges() {
        let mut f = Font::unknown();
        f.codes = Codes::Ranges(vec![(1, 0x00, 0x80), (2, 0x8140, 0xfefe)]);
        let c = f.codes(b"\x41\x81\x40\x42");
        assert_eq!(c.iter().map(|c| c.len).collect::<Vec<_>>(), [1, 2, 1]);
        assert_eq!(c[1].code, 0x8140);
        assert_eq!(c[1].hex(), "8140");
    }

    #[test]
    fn names_and_text() {
        let mut f = Font::unknown();
        f.default_names = Some(encoding::standard());
        let mut d = vec![None; 256];
        d[65] = Some("alpha".into());
        f.diff_names = d;
        let a = Code { code: 65, len: 1 };
        assert_eq!(f.text(a).as_deref(), Some("α"));
        assert_eq!(f.text(Code { code: 0x27, len: 1 }).as_deref(), Some("’"));
        assert_eq!(f.encoded_name(66), None);
    }
}
