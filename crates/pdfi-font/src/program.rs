//! Font programs: TrueType and OpenType (`skrifa`), bare CFF and
//! Type 1 (`hayro-font`). The format is told from the data, not from the
//! key that holds it (`/FontFile3 /Subtype /Type1C` streams that hold
//! OpenType are common).
//!
//! Outlines come out in text space for a font size of 1 (the font matrix
//! or `1 / unitsPerEm` applied), y up.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use hayro_font::{cff, type1};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::DrawSettings;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::cmap::PlatformId;
use skrifa::raw::tables::gsub::{SingleSubst, SubstitutionSubtables};
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `glyf` outlines in an sfnt.
    TrueType,
    /// CFF outlines in an sfnt (`OTTO`).
    OpenType,
    /// A bare CFF font (`Type1C`, `CIDFontType0C`).
    Cff,
    Type1,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::TrueType => "TrueType",
            Format::OpenType => "OpenType (CFF)",
            Format::Cff => "CFF",
            Format::Type1 => "Type 1",
        }
    }
}

/// One segment of a glyph outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Curve(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A glyph outline in text space for a font size of 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub segs: Vec<Seg>,
}

impl Outline {
    /// Bounding box `[x0, y0, x1, y1]` of the points (control points
    /// included).
    pub fn bbox(&self) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        let mut add = |x: f32, y: f32| {
            b = Some(match b {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        };
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) | Seg::Line(x, y) => add(x, y),
                Seg::Curve(a, b, c, d, e, f) => {
                    add(a, b);
                    add(c, d);
                    add(e, f);
                }
                Seg::Close => {}
            }
        }
        b
    }

    /// SVG path data, multiplied by `k` (y stays up).
    pub fn svg(&self, k: f32) -> String {
        use std::fmt::Write;
        let r = |v: f32| (v * k * 10.0).round() / 10.0;
        let mut s = String::new();
        for seg in &self.segs {
            let _ = match *seg {
                Seg::Move(x, y) => write!(s, "M{} {}", r(x), r(y)),
                Seg::Line(x, y) => write!(s, "L{} {}", r(x), r(y)),
                Seg::Curve(a, b, c, d, e, f) => {
                    write!(s, "C{} {} {} {} {} {}", r(a), r(b), r(c), r(d), r(e), r(f))
                }
                Seg::Close => write!(s, "Z"),
            };
        }
        s
    }
}

/// Collects outline segments through a matrix (quadratic curves become
/// cubic ones).
struct Builder {
    m: [f64; 6],
    out: Vec<Seg>,
    /// Current point, in font units.
    at: (f32, f32),
}

impl Builder {
    fn new(m: [f64; 6]) -> Self {
        Builder {
            m,
            out: Vec::new(),
            at: (0.0, 0.0),
        }
    }

    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (f64::from(x), f64::from(y));
        let m = &self.m;
        (
            (m[0] * x + m[2] * y + m[4]) as f32,
            (m[1] * x + m[3] * y + m[5]) as f32,
        )
    }

    fn move_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.out.push(Seg::Move(a, b));
        self.at = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.p(x, y);
        self.out.push(Seg::Line(a, b));
        self.at = (x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        let c1 = (x0 + 2.0 / 3.0 * (x1 - x0), y0 + 2.0 / 3.0 * (y1 - y0));
        let c2 = (x + 2.0 / 3.0 * (x1 - x), y + 2.0 / 3.0 * (y1 - y));
        self.curve_to(c1.0, c1.1, c2.0, c2.1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.p(x1, y1);
        let (c, d) = self.p(x2, y2);
        let (e, f) = self.p(x, y);
        self.out.push(Seg::Curve(a, b, c, d, e, f));
        self.at = (x, y);
    }

    fn close(&mut self) {
        self.out.push(Seg::Close);
    }

    fn finish(self) -> Outline {
        Outline { segs: self.out }
    }
}

impl skrifa::outline::OutlinePen for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        Builder::move_to(self, x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        Builder::line_to(self, x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        Builder::quad_to(self, x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        Builder::curve_to(self, x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        Builder::close(self);
    }
}

impl hayro_font::OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        Builder::move_to(self, x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        Builder::line_to(self, x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        Builder::quad_to(self, x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        Builder::curve_to(self, x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        Builder::close(self);
    }
}

fn matrix(m: hayro_font::Matrix) -> [f64; 6] {
    [
        m.sx.into(),
        m.ky.into(),
        m.kx.into(),
        m.sy.into(),
        m.tx.into(),
        m.ty.into(),
    ]
}

/// A parsed font program.
pub struct Program {
    pub format: Format,
    data: Cow<'static, [u8]>,
    type1: Option<type1::Table>,
    /// Number of glyphs (unknown for Type 1).
    pub glyphs: Option<u32>,
    /// sfnt `unitsPerEm`.
    pub units_per_em: Option<u16>,
    /// A CID-keyed CFF font (charset maps CIDs).
    pub cid_keyed: bool,
    /// Font units to text space.
    m: [f64; 6],
    /// Vertical alternates (`vrt2` or `vert`), read on first use.
    vertical: OnceLock<HashMap<u16, u16>>,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("format", &self.format)
            .field("len", &self.data.len())
            .field("glyphs", &self.glyphs)
            .finish()
    }
}

const OTTO: &[u8] = b"OTTO";
const CFF: Tag = Tag::new(b"CFF ");

impl Program {
    /// Parse a program, telling its format from the data.
    pub fn parse(data: Cow<'static, [u8]>) -> Result<Program, String> {
        let head = data.get(..4).unwrap_or(&[]);
        if head == [0, 1, 0, 0] || head == b"true" || head == OTTO || head == b"ttcf" {
            let raw =
                FontRef::from_index(&data, 0).map_err(|e| format!("not a valid sfnt font: {e}"))?;
            let (Ok(head), Ok(maxp), Ok(_)) = (raw.head(), raw.maxp(), raw.hhea()) else {
                // OpenType subsets in PDFs often lack tables a font
                // needs (`head`, `hhea`, `maxp`): read their CFF alone.
                return match raw.table_data(CFF) {
                    Some(c) => Program::parse(Cow::Owned(c.as_bytes().to_vec())),
                    None => Err("not a valid sfnt font: a required table is missing".into()),
                };
            };
            let upem = head.units_per_em().max(1);
            let cff_table = raw
                .table_data(CFF)
                .and_then(|c| cff::Table::parse(c.as_bytes()));
            let format = if cff_table.is_some() {
                Format::OpenType
            } else {
                Format::TrueType
            };
            let glyphs = Some(u32::from(maxp.num_glyphs()));
            let k = 1.0 / f64::from(upem);
            let cid_keyed = cff_table.is_some_and(|t| t.is_cid());
            return Ok(Program {
                format,
                glyphs,
                units_per_em: Some(upem),
                cid_keyed,
                m: [k, 0.0, 0.0, k, 0.0, 0.0],
                type1: None,
                data,
                vertical: OnceLock::new(),
            });
        }
        if head.first() == Some(&1) && head.len() == 4 {
            let t = cff::Table::parse(&data).ok_or("not a valid CFF font")?;
            return Ok(Program {
                format: Format::Cff,
                glyphs: Some(u32::from(t.number_of_glyphs())),
                units_per_em: None,
                cid_keyed: t.is_cid(),
                m: matrix(t.matrix()),
                type1: None,
                data,
                vertical: OnceLock::new(),
            });
        }
        // Type 1: PFB segments, or text starting with `%!` (possibly after
        // some junk).
        let start = if data.starts_with(&[0x80, 0x01]) {
            Some(0)
        } else {
            data.get(..1024.min(data.len()))
                .and_then(|h| h.windows(2).position(|w| w == b"%!"))
        };
        let Some(start) = start else {
            return Err("not a TrueType, OpenType, CFF or Type 1 font".into());
        };
        let t = type1::Table::parse(&data[start..]).ok_or("not a valid Type 1 font")?;
        Ok(Program {
            format: Format::Type1,
            glyphs: None,
            units_per_em: None,
            cid_keyed: false,
            m: matrix(t.matrix()),
            type1: Some(t),
            data,
            vertical: OnceLock::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The sfnt (TrueType and OpenType only).
    fn font(&self) -> Option<FontRef<'_>> {
        match self.format {
            Format::TrueType | Format::OpenType => FontRef::from_index(&self.data, 0).ok(),
            _ => None,
        }
    }

    /// The CFF table: the whole program, or an OpenType font's `CFF `.
    fn cff(&self) -> Option<cff::Table<'_>> {
        match self.format {
            Format::Cff => cff::Table::parse(&self.data),
            Format::OpenType => cff::Table::parse(self.font()?.table_data(CFF)?.as_bytes()),
            _ => None,
        }
    }

    /// The glyph that stands for `gid` in vertical writing: the font's
    /// vertical alternate (GSUB `vrt2`, else `vert`; single substitutions
    /// only), or `gid` itself.
    pub fn vertical_gid(&self, gid: u16) -> u16 {
        self.vertical
            .get_or_init(|| self.vertical_alternates())
            .get(&gid)
            .copied()
            .unwrap_or(gid)
    }

    fn vertical_alternates(&self) -> HashMap<u16, u16> {
        let mut m = HashMap::new();
        let Some(gsub) = self.font().and_then(|f| f.gsub().ok()) else {
            return m;
        };
        let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else {
            return m;
        };
        for tag in [b"vrt2", b"vert"] {
            let records = features.feature_records().iter();
            for r in records.filter(|r| r.feature_tag() == Tag::new(tag)) {
                let Ok(f) = r.feature(features.offset_data()) else {
                    continue;
                };
                for li in f.lookup_list_indices() {
                    let Ok(l) = lookups.lookups().get(usize::from(li.get())) else {
                        continue;
                    };
                    if let Ok(SubstitutionSubtables::Single(sts)) = l.subtables() {
                        for st in sts.iter().flatten() {
                            single(&st, &mut m);
                        }
                    }
                }
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    }

    /// The outline of a glyph by index.
    pub fn outline(&self, gid: u16) -> Option<Outline> {
        match self.format {
            Format::TrueType | Format::OpenType => {
                let font = self.font()?;
                let mut b = Builder::new(self.m);
                if let Some(g) = font.outline_glyphs().get(GlyphId::from(gid)) {
                    let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                    let _ = g.draw(settings, &mut b);
                }
                Some(b.finish())
            }
            Format::Cff => {
                let t = self.cff()?;
                let mut b = Builder::new(self.m);
                t.outline(hayro_font::GlyphId(gid), &mut b).ok()?;
                Some(b.finish())
            }
            Format::Type1 => None,
        }
    }

    /// The outline of a glyph by name (Type 1, or through the glyph names
    /// of the others).
    pub fn outline_by_name(&self, name: &str) -> Option<Outline> {
        match &self.type1 {
            Some(t) => {
                let mut b = Builder::new(self.m);
                t.outline(name, &mut b)?;
                Some(b.finish())
            }
            None => self.outline(self.gid_by_name(name)?),
        }
    }

    pub fn has_glyph_name(&self, name: &str) -> bool {
        match &self.type1 {
            Some(t) => t.outline(name, &mut Builder::new(self.m)).is_some(),
            None => self.gid_by_name(name).is_some(),
        }
    }

    pub fn gid_by_name(&self, name: &str) -> Option<u16> {
        match self.format {
            Format::TrueType | Format::OpenType => self
                .font()?
                .glyph_names()
                .iter()
                .find(|(_, n)| !n.is_synthesized() && n.as_str() == name)
                .and_then(|(g, _)| u16::try_from(g.to_u32()).ok()),
            Format::Cff => self.cff()?.glyph_index_by_name(name).map(|g| g.0),
            Format::Type1 => None,
        }
    }

    pub fn glyph_name(&self, gid: u16) -> Option<String> {
        match self.format {
            Format::TrueType | Format::OpenType => self
                .font()?
                .glyph_names()
                .get(GlyphId::from(gid))
                .filter(|n| !n.is_synthesized())
                .map(|n| n.as_str().to_owned()),
            Format::Cff => self
                .cff()?
                .glyph_name(hayro_font::GlyphId(gid))
                .map(String::from),
            Format::Type1 => None,
        }
    }

    /// Glyph name of a code in the font's built-in encoding (Type 1 and
    /// CFF).
    pub fn builtin_name(&self, code: u8) -> Option<String> {
        match &self.type1 {
            Some(t) => t
                .code_to_string(code)
                .filter(|n| *n != ".notdef")
                .map(String::from),
            None => self.glyph_name(self.builtin_gid(code)?),
        }
    }

    /// Glyph of a code in a CFF font's built-in encoding.
    pub fn builtin_gid(&self, code: u8) -> Option<u16> {
        self.cff()?
            .glyph_index(code)
            .map(|g| g.0)
            .filter(|&g| g != 0)
    }

    /// Whether the sfnt has a cmap subtable for (platform, encoding).
    pub fn has_cmap(&self, platform: u16, encoding: u16) -> bool {
        self.font().and_then(|f| f.cmap().ok()).is_some_and(|c| {
            c.encoding_records()
                .iter()
                .any(|r| platform_id(r.platform_id()) == platform && r.encoding_id() == encoding)
        })
    }

    /// Look a code up in the cmap subtable (platform, encoding).
    pub fn cmap(&self, platform: u16, encoding: u16, code: u32) -> Option<u16> {
        let c = self.font()?.cmap().ok()?;
        c.encoding_records()
            .iter()
            .filter(|r| platform_id(r.platform_id()) == platform && r.encoding_id() == encoding)
            .filter_map(|r| r.subtable(c.offset_data()).ok())
            .find_map(|s| s.map_codepoint(code))
            .and_then(|g| u16::try_from(g.to_u32()).ok())
            .filter(|&g| g != 0)
    }

    /// Whether the sfnt has a cmap at all.
    pub fn has_any_cmap(&self) -> bool {
        self.font()
            .and_then(|f| f.cmap().ok())
            .is_some_and(|c| !c.encoding_records().is_empty())
    }

    /// The glyph of a CID: the charset of CID-keyed CFF fonts, the CID
    /// itself otherwise.
    pub fn gid_of_cid(&self, cid: u32) -> Option<u16> {
        if self.cid_keyed {
            return self
                .cff()?
                .glyph_index_by_cid(u16::try_from(cid).ok()?)
                .map(|g| g.0);
        }
        u16::try_from(cid).ok()
    }

    /// Advance width of a glyph in text space for a font size of 1.
    pub fn advance(&self, gid: u16) -> Option<f64> {
        match self.format {
            Format::TrueType | Format::OpenType => {
                let f = self.font()?;
                f.hmtx().ok()?;
                let metrics = f.glyph_metrics(Size::unscaled(), LocationRef::default());
                let w = metrics.advance_width(GlyphId::from(gid))?;
                Some(f64::from(w) * self.m[0])
            }
            Format::Cff => {
                let w = self.cff()?.glyph_width(hayro_font::GlyphId(gid))?;
                Some(f64::from(w) * self.m[0])
            }
            Format::Type1 => None,
        }
    }
}

fn platform_id(p: PlatformId) -> u16 {
    match p {
        PlatformId::Unicode => 0,
        PlatformId::Macintosh => 1,
        PlatformId::ISO => 2,
        PlatformId::Windows => 3,
        PlatformId::Custom => 4,
        _ => u16::MAX,
    }
}

/// Add the substitutions of a single-substitution subtable (the first
/// one found for a glyph wins).
fn single(s: &SingleSubst<'_>, m: &mut HashMap<u16, u16>) {
    let mut add = |g: GlyphId, to: Option<u32>| {
        let (Ok(g), Some(Ok(to))) = (u16::try_from(g.to_u32()), to.map(u16::try_from)) else {
            return;
        };
        m.entry(g).or_insert(to);
    };
    match s {
        SingleSubst::Format1(t) => {
            let Ok(coverage) = t.coverage() else { return };
            let delta = t.delta_glyph_id() as u16;
            for g in coverage.iter() {
                add(g, Some(u32::from((g.to_u32() as u16).wrapping_add(delta))));
            }
        }
        SingleSubst::Format2(t) => {
            let Ok(coverage) = t.coverage() else { return };
            let subs = t.substitute_glyph_ids();
            for (i, g) in coverage.iter().enumerate() {
                add(g, subs.get(i).map(|s| u32::from(s.get().to_u16())));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_quads() {
        let mut b = Builder::new([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
        b.move_to(0.0, 0.0);
        b.quad_to(3.0, 3.0, 6.0, 0.0);
        b.close();
        let o = b.finish();
        assert_eq!(o.segs[0], Seg::Move(0.0, 0.0));
        assert_eq!(o.segs[1], Seg::Curve(1.0, 1.0, 2.0, 1.0, 3.0, 0.0));
        assert_eq!(o.svg(2.0), "M0 0C2 2 4 2 6 0Z");
        assert_eq!(o.bbox(), Some([0.0, 0.0, 3.0, 1.0]));
    }

    #[test]
    fn rejects_junk() {
        assert!(Program::parse(Cow::Borrowed(b"hello world")).is_err());
        assert!(Program::parse(Cow::Borrowed(b"\0\x01\0\0junk")).is_err());
    }
}
