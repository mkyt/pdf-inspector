//! `pdfi font`: a font dictionary, its program and its glyphs.

use anyhow::{Result, bail};
use pdfi_content::font::{self, CidToGid, FontKind, GlyphSource};
use pdfi_syntax::{ObjId, PdfFile};

pub fn font(file: &PdfFile, num: u32, glyphs: bool) -> Result<()> {
    let id = ObjId::new(num, 0);
    let held = file.get(id)?;
    let Some(d) = held.node.obj.as_dict() else {
        bail!("{num} 0 obj is not a dictionary");
    };
    let f = font::load(file, Some(id), d);
    let opt = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
    println!("{num} 0 obj  /{}  {}", opt(&f.subtype), opt(&f.base_font));
    if let Some(dd) = f.descendant {
        println!("  descendant {dd}  /{}", opt(&f.cid_subtype));
    }
    println!(
        "  encoding   {}{}",
        opt(&f.encoding),
        if f.vertical { "  (vertical)" } else { "" }
    );
    if let Some(m) = &f.cid_to_gid {
        match m {
            CidToGid::Identity => println!("  CIDToGID   Identity"),
            CidToGid::Map(v) => println!("  CIDToGID   map of {} CIDs", v.len()),
        }
    }
    if let Some(u) = f.to_unicode {
        println!("  ToUnicode  {u} ({} entries)", f.to_unicode_entries);
    }
    if let Some(fl) = f.flags {
        println!(
            "  flags      {fl}{}",
            if f.symbolic { " (symbolic)" } else { "" }
        );
    }
    match &f.program {
        Some(p) => println!(
            "  program    /{}{} {}",
            p.key,
            p.subtype
                .as_ref()
                .map(|s| format!(" /{s}"))
                .unwrap_or_default(),
            p.id.map(|i| i.to_string()).unwrap_or_default()
        ),
        None if f.kind != FontKind::Type3 => println!("  program    not embedded"),
        None => {}
    }
    let g = f.glyphs(file);
    match &g.source {
        GlyphSource::Embedded(p) => println!(
            "  glyphs     {} program, {} bytes{}{}",
            p.format.name(),
            p.len(),
            p.glyphs
                .map(|n| format!(", {n} glyphs"))
                .unwrap_or_default(),
            if p.cid_keyed { ", CID-keyed" } else { "" }
        ),
        GlyphSource::Substitute { name, .. } => println!("  glyphs     substituted by {name}"),
        GlyphSource::Procedures => println!("  glyphs     Type 3 procedures"),
        GlyphSource::None => println!("  glyphs     none (drawn as boxes)"),
    }
    if f.approximate {
        println!("  widths     partly guessed");
    }
    for i in f.issues.iter().chain(&g.issues) {
        println!(
            "  {:<7} {}",
            format!("{:?}", i.severity).to_lowercase(),
            i.message
        );
    }
    if !glyphs {
        return Ok(());
    }
    println!("  code      cid    gid  name                 text  width  outline");
    for c in f.listed_codes(file) {
        let r = f.glyph(file, c);
        let text = f.text(c).unwrap_or_default();
        let proc_ = (f.kind == FontKind::Type3)
            .then(|| f.char_proc(file, c).and_then(|p| p.0))
            .flatten();
        if r.name.is_none() && r.gid.is_none() && text.is_empty() && f.glyph_width(c) == 0.0 {
            continue;
        }
        let outline = match proc_ {
            Some(p) => format!("procedure {p}"),
            None => f
                .outline(file, c)
                .and_then(|o| o.bbox())
                .map(|b| {
                    format!(
                        "[{:.0} {:.0} {:.0} {:.0}]",
                        b[0] * 1000.0,
                        b[1] * 1000.0,
                        b[2] * 1000.0,
                        b[3] * 1000.0
                    )
                })
                .unwrap_or_else(|| "-".into()),
        };
        println!(
            "  {:<8} {:>5} {:>6}  {:<20} {:<5} {:>6}  {outline}",
            c.hex(),
            r.cid.map(|v| v.to_string()).unwrap_or_default(),
            r.gid.map(|v| v.to_string()).unwrap_or_default(),
            r.name.unwrap_or_default(),
            text.escape_debug().to_string(),
            f.glyph_width(c),
        );
    }
    Ok(())
}
