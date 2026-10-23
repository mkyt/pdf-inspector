//! Requests about fonts (pdfi-font): the dictionaries, the program and the
//! glyph each code selects.

use std::collections::BTreeMap;
use std::sync::Arc;

use pdfi_content::Target;
use pdfi_content::font::{self as pf, CidToGid, Code, Font, GlyphSource, Outline};
use pdfi_syntax::{self as syn, DictExt, ObjId};

use crate::dto::*;
use crate::model::rect;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, lock, page};

/// Glyph space units per em in `GlyphView` outlines.
const UNITS: f32 = 1000.0;

/// `/Flags` bits (1-based, ISO 32000 table 121).
const FLAGS: [(u32, &str); 9] = [
    (1, "FixedPitch"),
    (2, "Serif"),
    (3, "Symbolic"),
    (4, "Script"),
    (6, "Nonsymbolic"),
    (7, "Italic"),
    (17, "AllCap"),
    (18, "SmallCap"),
    (19, "ForceBold"),
];

fn load(d: &Doc, id: ObjRef) -> Result<Arc<Font>, ApiError> {
    let oid: ObjId = id.into();
    let held = d
        .file
        .get(oid)
        .map_err(|e| ApiError::new(ErrorCode::NotFound, e.to_string()))?;
    let dict = held.node.obj.as_dict().ok_or_else(|| {
        ApiError::new(
            ErrorCode::BadRequest,
            format!("{oid} obj is not a font dictionary"),
        )
    })?;
    if dict.get_key("Subtype").is_none() && dict.get_key("BaseFont").is_none() {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            format!("{oid} obj is not a font dictionary"),
        ));
    }
    if dict
        .get_name("Subtype")
        .is_some_and(|s| s.as_bytes().starts_with(b"CIDFontType"))
    {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            format!("{oid} obj is a CID font: open the Type 0 font that uses it"),
        ));
    }
    Ok(d.content.font(&d.file, Some(oid), dict))
}

fn issue(f: &Font, i: &pf::FontIssue) -> DiagnosticInfo {
    DiagnosticInfo {
        severity: match i.severity {
            syn::Severity::Info => Severity::Info,
            syn::Severity::Warning => Severity::Warning,
            syn::Severity::Error => Severity::Error,
        },
        code: "Font".into(),
        message: i.message.clone(),
        obj: f.id.map(Into::into),
        path: None,
        span: None,
    }
}

pub fn detail(d: &Doc, id: ObjRef) -> Result<FontDetail, ApiError> {
    let f = load(d, id)?;
    let g = f.glyphs(&d.file);
    let (glyph_source, substitute) = match &g.source {
        GlyphSource::Embedded(_) => (GlyphSourceKind::Embedded, None),
        GlyphSource::Substitute { name, .. } => {
            (GlyphSourceKind::Substitute, Some(name.to_string()))
        }
        GlyphSource::Procedures => (GlyphSourceKind::Procedures, None),
        GlyphSource::None => (GlyphSourceKind::None, None),
    };
    let program = f.program.as_ref().map(|p| {
        let prog = match &g.source {
            GlyphSource::Embedded(prog) => Some(prog),
            _ => None,
        };
        FontProgramInfo {
            stream: p.id.map(Into::into),
            key: p.key.into(),
            subtype: p.subtype.clone(),
            format: prog.map(|p| p.format.name().into()),
            len: prog.map_or(0, |p| p.len() as u64),
            glyphs: prog.and_then(|p| p.glyphs),
            units_per_em: prog.and_then(|p| p.units_per_em).map(u32::from),
            cid_keyed: prog.is_some_and(|p| p.cid_keyed),
        }
    });
    // Text space per glyph space unit.
    let glyph_unit = if f.scale != 0.0 { f.scale.abs() } else { 0.001 };
    Ok(FontDetail {
        id,
        kind: match f.kind {
            pf::FontKind::Simple => FontKind::Simple,
            pf::FontKind::Type0 => FontKind::Type0,
            pf::FontKind::Type3 => FontKind::Type3,
        },
        subtype: f.subtype.clone(),
        base_font: f.base_font.clone(),
        descendant: f.descendant.map(Into::into),
        cid_subtype: f.cid_subtype.clone(),
        descriptor: f.descriptor.map(Into::into),
        encoding: f.encoding.clone(),
        vertical: f.vertical,
        to_unicode: f.to_unicode.map(Into::into),
        to_unicode_entries: f.to_unicode_entries as u32,
        cid_to_gid: f.cid_to_gid.as_ref().map(|m| match m {
            CidToGid::Identity => "Identity".into(),
            CidToGid::Map(v) => format!("map of {} CIDs", v.len()),
        }),
        program,
        glyph_source,
        substitute,
        flags: f.flags,
        flag_names: f
            .flags
            .map(|fl| {
                FLAGS
                    .iter()
                    .filter(|(bit, _)| fl & (1 << (bit - 1)) != 0)
                    .map(|(_, n)| n.to_string())
                    .collect()
            })
            .unwrap_or_default(),
        symbolic: f.symbolic,
        ascent: f.ascent / glyph_unit,
        descent: f.descent / glyph_unit,
        font_matrix: f.type3.as_ref().map(|t| t.matrix.to_vec()),
        approximate: f.approximate,
        codes: f.listed_codes(&d.file).len() as u32,
        issues: f
            .issues
            .iter()
            .chain(&g.issues)
            .map(|i| issue(&f, i))
            .collect(),
    })
}

fn bbox(o: &Outline) -> Option<Rect> {
    o.bbox().map(|b| {
        // Tenths of a unit, as the path data.
        let u = |v: f32| (f64::from(v * UNITS) * 10.0).round() / 10.0;
        Rect {
            x0: u(b[0]),
            y0: u(b[1]),
            x1: u(b[2]),
            y1: u(b[3]),
        }
    })
}

fn glyph(d: &Doc, f: &Font, c: Code) -> GlyphView {
    let file = &d.file;
    let r = f.glyph(file, c);
    let outline = f.outline(file, c);
    GlyphView {
        code: c.code,
        bytes: c.hex(),
        cid: r.cid,
        gid: r.gid.map(u32::from),
        name: r.name,
        text: f.text(c),
        width: f.glyph_width(c),
        outline: outline
            .as_ref()
            .filter(|o| !o.segs.is_empty())
            .map(|o| o.svg(UNITS)),
        bbox: outline.as_deref().and_then(bbox),
        procedure: f.char_proc(file, c).and_then(|(id, _)| id).map(Into::into),
    }
}

pub fn glyphs(d: &Doc, id: ObjRef, range: Range) -> Result<Page<GlyphView>, ApiError> {
    let f = load(d, id)?;
    let codes = f.listed_codes(&d.file);
    Ok(page(codes.len() as u64, range, |o, n| {
        codes[o..o + n].iter().map(|&c| glyph(d, &f, c)).collect()
    }))
}

pub fn program_glyphs(d: &Doc, id: ObjRef, range: Range) -> Result<Page<ProgramGlyph>, ApiError> {
    let f = load(d, id)?;
    let file = &d.file;
    let p = match &f.glyphs(file).source {
        GlyphSource::Embedded(p) => p.clone(),
        _ => return Ok(page(0, range, |_, _| Vec::new())),
    };
    let total = p.glyphs.unwrap_or(0);
    Ok(page(total.into(), range, |o, n| {
        // The codes that select each glyph of the page.
        let mut codes: Vec<Vec<Code>> = vec![Vec::new(); n];
        for c in f.listed_codes(file) {
            if let Some(i) = f
                .glyph(file, c)
                .gid
                .and_then(|g| usize::from(g).checked_sub(o))
                .filter(|&i| i < n)
            {
                codes[i].push(c);
            }
        }
        codes
            .into_iter()
            .enumerate()
            .map(|(i, cs)| {
                let gid = (o + i) as u16;
                let outline = p.outline(gid);
                ProgramGlyph {
                    gid: gid.into(),
                    name: p.glyph_name(gid),
                    text: cs.iter().find_map(|&c| f.text(c)),
                    codes: cs.iter().map(|c| c.hex()).collect(),
                    width: p.advance(gid).map_or(0.0, |w| w * f64::from(UNITS)),
                    outline: outline
                        .as_ref()
                        .filter(|o| !o.segs.is_empty())
                        .map(|o| o.svg(UNITS)),
                    bbox: outline.as_ref().and_then(bbox),
                }
            })
            .collect()
    }))
}

/// Characters of text kept per use.
const MAX_USE_TEXT: usize = 300;

/// The text a font shows, gathered page by page by a `FontUses` job.
#[derive(Default)]
pub(crate) struct FontUsage {
    pub scanned: u32,
    pub pages: u32,
    pub uses: Vec<FontUse>,
    /// By (length, code).
    pub codes: BTreeMap<(u8, u32), u64>,
}

impl FontUsage {
    /// Add what `font` shows on a page (also inside its form XObjects).
    pub fn scan_page(&mut self, d: &Doc, font: ObjId, index: u32) {
        self.scanned += 1;
        let Ok(a) = d
            .content
            .on(d.model.on(&d.file))
            .analyze_once(Target::Page(index.into()))
        else {
            return;
        };
        let before = self.uses.len();
        for sh in a.shows.iter().filter(|sh| sh.font == Some(font)) {
            for c in &sh.codes {
                *self.codes.entry(c.key()).or_default() += 1;
            }
            let mut text = sh.text.clone();
            if let Some((cut, _)) = text.char_indices().nth(MAX_USE_TEXT) {
                text.truncate(cut);
                text.push('…');
            }
            self.uses.push(FontUse {
                page: index,
                op: sh.op,
                form: sh.inner.map(|(id, _)| id.into()),
                form_op: sh.inner.map(|(_, op)| op),
                text,
                codes: sh.codes.len() as u32,
                bbox: sh.bbox.map(rect),
                hidden: sh.hidden,
            });
        }
        if self.uses.len() > before {
            self.pages += 1;
        }
    }
}

/// Pages a `FontUses` job goes through: those the inventory found using
/// the font, or every page.
pub(crate) fn pages_to_scan(d: &Doc, font: ObjId) -> Vec<u32> {
    if let Some(inv) = lock(&d.inventory).clone() {
        if let Some(f) = inv.fonts.iter().find(|f| f.id == font) {
            return f.pages.clone();
        }
    }
    let n = d.model.on(&d.file).page_count().min(u32::MAX.into()) as u32;
    (0..n).collect()
}

pub fn uses(d: &Doc, id: ObjRef, range: Range) -> FontUsesView {
    let u = lock(&d.font_uses).get(&id.into()).cloned();
    let Some(u) = u else {
        return FontUsesView {
            scanned: false,
            pages_scanned: 0,
            pages: 0,
            codes: Vec::new(),
            uses: page(0, range, |_, _| Vec::new()),
        };
    };
    FontUsesView {
        scanned: true,
        pages_scanned: u.scanned,
        pages: u.pages,
        codes: u
            .codes
            .iter()
            .map(|(&(len, code), &count)| CodeCount {
                bytes: Code { code, len }.hex(),
                count,
            })
            .collect(),
        uses: page(u.uses.len() as u64, range, |o, n| u.uses[o..o + n].to_vec()),
    }
}
