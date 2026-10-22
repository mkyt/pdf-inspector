//! Requests about fonts (pdfi-font): the dictionaries, the program and the
//! glyph each code selects.

use std::sync::Arc;

use pdfi_content::font::{self as pf, CidToGid, Code, Font, GlyphSource};
use pdfi_syntax::{self as syn, DictExt, ObjId};

use crate::dto::*;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, page};

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
        bbox: outline.as_ref().and_then(|o| o.bbox()).map(|b| {
            // Tenths of a unit, as the path data.
            let u = |v: f32| (f64::from(v * UNITS) * 10.0).round() / 10.0;
            Rect {
                x0: u(b[0]),
                y0: u(b[1]),
                x1: u(b[2]),
                y1: u(b[3]),
            }
        }),
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
