//! Font programs: TrueType and OpenType (`ttf-parser`), bare CFF and
//! Type 1 (`hayro-font`). The format is told from the data, not from the
//! key that holds it (`/FontFile3 /Subtype /Type1C` streams that hold
//! OpenType are common).
//!
//! Outlines come out in text space for a font size of 1 (the font matrix
//! or `1 / unitsPerEm` applied), y up.

use std::borrow::Cow;

use hayro_font::{cff, type1};
use ttf_parser::{Face, GlyphId, PlatformId, Tag};

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

impl ttf_parser::OutlineBuilder for Builder {
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

impl Program {
    /// Parse a program, telling its format from the data.
    pub fn parse(data: Cow<'static, [u8]>) -> Result<Program, String> {
        let head = data.get(..4).unwrap_or(&[]);
        if head == [0, 1, 0, 0] || head == b"true" || head == OTTO || head == b"ttcf" {
            let face = match Face::parse(&data, 0) {
                Ok(f) => f,
                Err(e) => {
                    // OpenType subsets in PDFs often lack tables a font
                    // needs (`head`, `hhea`, `maxp`): read their CFF alone.
                    let cff = ttf_parser::RawFace::parse(&data, 0)
                        .ok()
                        .and_then(|r| r.table(Tag::from_bytes(b"CFF ")))
                        .map(<[u8]>::to_vec);
                    return match cff {
                        Some(c) => Program::parse(Cow::Owned(c)),
                        None => Err(format!("not a valid sfnt font: {e}")),
                    };
                }
            };
            let upem = face.units_per_em().max(1);
            let format = if face.tables().cff.is_some() {
                Format::OpenType
            } else {
                Format::TrueType
            };
            let glyphs = Some(u32::from(face.number_of_glyphs()));
            let k = 1.0 / f64::from(upem);
            let cid_keyed = face
                .raw_face()
                .table(Tag::from_bytes(b"CFF "))
                .and_then(cff::Table::parse)
                .is_some_and(|t| t.is_cid());
            return Ok(Program {
                format,
                glyphs,
                units_per_em: Some(upem),
                cid_keyed,
                m: [k, 0.0, 0.0, k, 0.0, 0.0],
                type1: None,
                data,
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
        })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn face(&self) -> Option<Face<'_>> {
        Face::parse(&self.data, 0).ok()
    }

    /// The CFF table: the whole program, or an OpenType font's `CFF `.
    fn cff(&self) -> Option<cff::Table<'_>> {
        match self.format {
            Format::Cff => cff::Table::parse(&self.data),
            Format::OpenType => self
                .face()?
                .raw_face()
                .table(Tag::from_bytes(b"CFF "))
                .and_then(cff::Table::parse),
            _ => None,
        }
    }

    /// The outline of a glyph by index.
    pub fn outline(&self, gid: u16) -> Option<Outline> {
        match self.format {
            Format::TrueType | Format::OpenType => {
                let face = self.face()?;
                let mut b = Builder::new(self.m);
                face.outline_glyph(GlyphId(gid), &mut b);
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
            Format::TrueType | Format::OpenType => {
                self.face()?.glyph_index_by_name(name).map(|g| g.0)
            }
            Format::Cff => self.cff()?.glyph_index_by_name(name).map(|g| g.0),
            Format::Type1 => None,
        }
    }

    pub fn glyph_name(&self, gid: u16) -> Option<String> {
        match self.format {
            Format::TrueType | Format::OpenType => {
                self.face()?.glyph_name(GlyphId(gid)).map(String::from)
            }
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
        self.face().is_some_and(|f| {
            f.tables().cmap.is_some_and(|c| {
                c.subtables
                    .into_iter()
                    .any(|s| platform_id(s.platform_id) == platform && s.encoding_id == encoding)
            })
        })
    }

    /// Look a code up in the cmap subtable (platform, encoding).
    pub fn cmap(&self, platform: u16, encoding: u16, code: u32) -> Option<u16> {
        let f = self.face()?;
        let c = f.tables().cmap?;
        c.subtables
            .into_iter()
            .filter(|s| platform_id(s.platform_id) == platform && s.encoding_id == encoding)
            .find_map(|s| s.glyph_index(code))
            .map(|g| g.0)
            .filter(|&g| g != 0)
    }

    /// Whether the sfnt has a cmap at all.
    pub fn has_any_cmap(&self) -> bool {
        self.face()
            .and_then(|f| f.tables().cmap)
            .is_some_and(|c| !c.subtables.is_empty())
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
                let f = self.face()?;
                let w = f.glyph_hor_advance(GlyphId(gid))?;
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
        PlatformId::Iso => 2,
        PlatformId::Windows => 3,
        PlatformId::Custom => 4,
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
