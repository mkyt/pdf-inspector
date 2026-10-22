use std::sync::Arc;

use pdfi_font::{CidToGid, Code, Font, FontKind, Format, GlyphSource, load};
use pdfi_syntax::{MemSource, ObjId, OpenOptions, PdfFile, Severity};

fn open(name: &str) -> PdfFile {
    let p = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    PdfFile::open(
        Arc::new(MemSource::new(std::fs::read(&p).unwrap())),
        OpenOptions::default(),
    )
    .unwrap()
}

fn font(file: &PdfFile, num: u32) -> Font {
    let id = ObjId::new(num, 0);
    let held = file.get(id).unwrap();
    load(file, Some(id), held.node.obj.as_dict().unwrap())
}

fn one(code: u32) -> Code {
    Code { code, len: 1 }
}

/// The fixture's square glyph: 100..600 × 0..700 in a 1000-unit em.
#[track_caller]
fn assert_square(f: &Font, file: &PdfFile, c: Code) {
    let o = f.outline(file, c).expect("an outline");
    let b = o.bbox().unwrap();
    let want = [0.1, 0.0, 0.6, 0.7];
    assert!(
        b.iter().zip(want).all(|(a, w)| (a - w).abs() < 1e-3),
        "{:?}: {b:?}",
        f.base_font
    );
}

#[track_caller]
fn assert_embedded(f: &Font, file: &PdfFile, format: Format) {
    match &f.glyphs(file).source {
        GlyphSource::Embedded(p) => assert_eq!(p.format, format, "{:?}", f.base_font),
        other => panic!("{:?}: {other:?}", f.base_font),
    }
}

#[test]
fn truetype() {
    let file = open("fonts.pdf");
    let f = font(&file, 21);
    assert_embedded(&f, &file, Format::TrueType);
    assert_square(&f, &file, one(b'A'.into()));
    let g = f.glyph(&file, one(b'A'.into()));
    assert_eq!((g.gid, g.name.as_deref()), (Some(1), Some("A")));
    assert_eq!(f.text(one(b'A'.into())).as_deref(), Some("A"));
    assert!(f.issues.is_empty() && f.glyphs(&file).issues.is_empty());

    // Symbolic, no /Encoding: the (3, 0) cmap at 0xF000 + code.
    let s = font(&file, 22);
    assert!(s.symbolic);
    assert_eq!(s.glyph(&file, one(b'A'.into())).gid, Some(1));
    assert_square(&s, &file, one(b'A'.into()));
}

#[test]
fn cff_and_opentype() {
    let file = open("fonts.pdf");
    let f = font(&file, 23);
    assert_embedded(&f, &file, Format::Cff);
    assert_eq!(
        f.encoding.as_deref(),
        Some("WinAnsiEncoding + 1 differences")
    );
    assert_square(&f, &file, one(b'A'.into()));
    // /Differences [66 /A].
    assert_eq!(f.encoded_name(66), Some("A"));
    assert_square(&f, &file, one(b'B'.into()));
    assert_eq!(f.glyph(&file, one(b'B'.into())).gid, Some(1));

    let o = font(&file, 24);
    assert_embedded(&o, &file, Format::OpenType);
    assert_square(&o, &file, one(b'A'.into()));
}

#[test]
fn type1() {
    let file = open("fonts.pdf");
    let f = font(&file, 26);
    assert_embedded(&f, &file, Format::Type1);
    let g = f.glyph(&file, one(b'A'.into()));
    assert_eq!(g.name.as_deref(), Some("A"));
    assert_square(&f, &file, one(b'A'.into()));
    assert!(f.outline(&file, one(b'B'.into())).is_none());
}

#[test]
fn cid_fonts() {
    let file = open("fonts.pdf");
    let f = font(&file, 25);
    assert_eq!(f.kind, FontKind::Type0);
    assert_eq!(f.descendant, Some(ObjId::new(35, 0)));
    assert_eq!(f.cid_to_gid, Some(CidToGid::Map(vec![0, 0, 0, 0, 0, 1])));
    let c = f.codes(b"\x00\x05")[0];
    assert_eq!(c, Code { code: 5, len: 2 });
    let g = f.glyph(&file, c);
    assert_eq!((g.cid, g.gid), (Some(5), Some(1)));
    assert_square(&f, &file, c);
    assert_eq!(f.text(c).as_deref(), Some("X"));
    assert_eq!(f.glyph_width(c), 700.0);
    // Identity-H: the CIDs /CIDToGIDMap maps.
    assert_eq!(f.listed_codes(&file).len(), 6);

    // Not embedded: nothing draws it.
    let m = font(&file, 29);
    assert!(matches!(m.glyphs(&file).source, GlyphSource::None));
    assert!(!m.has_outlines(&file));
    assert_eq!(m.glyphs(&file).issues[0].severity, Severity::Warning);
}

#[test]
fn type3() {
    let file = open("fonts.pdf");
    let f = font(&file, 27);
    assert_eq!(f.kind, FontKind::Type3);
    assert!(matches!(f.glyphs(&file).source, GlyphSource::Procedures));
    let t = f.type3.as_ref().unwrap();
    assert_eq!(t.matrix, [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
    let (id, o) = f.char_proc(&file, one(b'a'.into())).unwrap();
    assert_eq!(id, Some(ObjId::new(50, 0)));
    assert!(o.as_stream().is_some());
    assert_eq!(
        f.char_proc(&file, one(b'b'.into())).unwrap().0,
        Some(ObjId::new(51, 0))
    );
    assert!(f.char_proc(&file, one(b'c'.into())).is_none());
    assert_eq!(f.glyph(&file, one(b'a'.into())).name.as_deref(), Some("sq"));
}

#[test]
fn standard_14_substitute() {
    let file = open("fonts.pdf");
    let f = font(&file, 28);
    match &f.glyphs(&file).source {
        GlyphSource::Substitute { name, .. } => assert_eq!(*name, "Foxit Sans"),
        other => panic!("{other:?}"),
    }
    // AFM widths, no guessing.
    assert!(!f.approximate);
    assert_eq!(f.glyph_width(one(b'H'.into())), 722.0);
    assert_eq!(f.glyph_width(one(0x80)), 556.0);
    let o = f.outline(&file, one(b'H'.into())).unwrap();
    let b = o.bbox().unwrap();
    assert!(
        b[0] > 0.05 && b[2] < 0.7 && (b[3] - 0.718).abs() < 0.02,
        "{b:?}"
    );
    assert_eq!(f.glyph(&file, one(0x80)).name.as_deref(), Some("Euro"));
}

#[test]
fn widths_and_text_of_the_model_fixture() {
    // Helvetica without /Encoding or /Widths: standard encoding, AFM widths.
    let file = open("base.pdf");
    let f = font(&file, 6);
    assert_eq!(f.glyph_width(one(b'P'.into())), 667.0);
    assert_eq!(f.text(one(0x27)).as_deref(), Some("’"));
    assert!(f.has_outlines(&file));
}

#[test]
fn damaged_programs_do_not_panic() {
    use pdfi_font::Program;
    use std::borrow::Cow;
    let file = open("fonts.pdf");
    let mut programs = Vec::new();
    for num in [61, 62, 63, 64, 66] {
        let id = ObjId::new(num, 0);
        let held = file.get(id).unwrap();
        let s = held.node.obj.as_stream().unwrap();
        programs.push(file.decode_stream(id, s, None).data);
    }
    // A small linear congruential generator: the same damage every run.
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed >> 8
    };
    for p in &programs {
        for round in 0..300 {
            let mut d = p.clone();
            if round % 3 == 0 {
                d.truncate(next() as usize % d.len());
            } else {
                for _ in 0..1 + round % 8 {
                    let i = next() as usize % d.len();
                    d[i] = next() as u8;
                }
            }
            let Ok(prog) = Program::parse(Cow::Owned(d)) else {
                continue;
            };
            for gid in 0..4 {
                let _ = prog.outline(gid);
                let _ = prog.glyph_name(gid);
            }
            let _ = prog.outline_by_name("A");
            let _ = prog.builtin_name(b'A');
            let _ = prog.cmap(3, 0, 0xf041);
        }
    }
}
