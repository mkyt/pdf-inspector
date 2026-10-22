//! Fonts that draw fonts which are not embedded. Simple fonts: the Foxit
//! fonts that PDFium ships (BSD-3-Clause, see `assets/LICENSE_FOXIT`),
//! metric compatible with the standard 14 fonts. They are bare CFF fonts
//! despite the `.pfb` names, compiled in (265 KB) so the browser build
//! draws the same. CID fonts: Harano Aji Mincho and Gothic (OFL-1.1, see
//! `assets/LICENSE_HARANOAJI`), Japanese fonts made from Source Han Serif
//! and Sans with the glyphs in Adobe-Japan1 order, so Japan1 CIDs pick
//! them directly (compiled in too, 10.5 MB).

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use crate::encoding::strip_subset;
use crate::program::Program;

struct Face {
    name: &'static str,
    data: &'static [u8],
}

macro_rules! face {
    ($name:literal, $file:literal) => {
        Face {
            name: $name,
            data: include_bytes!(concat!("../assets/", $file)),
        }
    };
}

/// Sans, serif, fixed by (bold, italic), then symbol and dingbats.
static FACES: [Face; 14] = [
    face!("Foxit Sans", "FoxitSans.pfb"),
    face!("Foxit Sans Italic", "FoxitSansItalic.pfb"),
    face!("Foxit Sans Bold", "FoxitSansBold.pfb"),
    face!("Foxit Sans Bold Italic", "FoxitSansBoldItalic.pfb"),
    face!("Foxit Serif", "FoxitSerif.pfb"),
    face!("Foxit Serif Italic", "FoxitSerifItalic.pfb"),
    face!("Foxit Serif Bold", "FoxitSerifBold.pfb"),
    face!("Foxit Serif Bold Italic", "FoxitSerifBoldItalic.pfb"),
    face!("Foxit Fixed", "FoxitFixed.pfb"),
    face!("Foxit Fixed Italic", "FoxitFixedItalic.pfb"),
    face!("Foxit Fixed Bold", "FoxitFixedBold.pfb"),
    face!("Foxit Fixed Bold Italic", "FoxitFixedBoldItalic.pfb"),
    face!("Foxit Symbol", "FoxitSymbol.pfb"),
    face!("Foxit Dingbats", "FoxitDingbats.pfb"),
];

static PARSED: [OnceLock<Option<Arc<Program>>>; 14] = [const { OnceLock::new() }; 14];

/// Mincho (serif) and Gothic (sans serif), Adobe-Japan1-7.
static CJK_FACES: [Face; 2] = [
    face!("Harano Aji Mincho", "HaranoAjiMincho-Regular.otf"),
    face!("Harano Aji Gothic", "HaranoAjiGothic-Medium.otf"),
];

static CJK_PARSED: [OnceLock<Option<Arc<Program>>>; 2] = [const { OnceLock::new() }; 2];

/// What is known of a font that is not embedded.
#[derive(Clone, Copy, Debug, Default)]
pub struct Traits<'a> {
    pub base_font: Option<&'a str>,
    /// `/FontDescriptor /Flags`.
    pub flags: i64,
    pub weight: Option<f64>,
    pub italic_angle: f64,
}

const FIXED_PITCH: i64 = 1;
const SERIF: i64 = 1 << 1;
const ITALIC: i64 = 1 << 6;
const FORCE_BOLD: i64 = 1 << 18;

/// The face that stands in for a font, by its name and descriptor.
fn choose(t: &Traits<'_>) -> usize {
    let name = t
        .base_font
        .map(|n| {
            strip_subset(n)
                .replace([' ', '-', ',', '_'], "")
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    if name.starts_with("symbol") {
        return 12;
    }
    if name.contains("dingbats") {
        return 13;
    }
    let has = |w: &[&str]| w.iter().any(|w| name.contains(w));
    let family = if name.starts_with("courier") || has(&["mono", "consola", "fixed"]) {
        8
    } else if name.starts_with("helvetica") || name.starts_with("arial") || has(&["sans", "gothic"])
    {
        0
    } else if name.starts_with("times")
        || has(&[
            "serif", "roman", "georgia", "garamond", "mincho", "song", "ming",
        ])
    {
        4
    } else if t.flags & FIXED_PITCH != 0 {
        8
    } else if t.flags & SERIF != 0 {
        4
    } else {
        0
    };
    let bold = has(&["bold", "black", "heavy", "demi"])
        || t.flags & FORCE_BOLD != 0
        || t.weight.is_some_and(|w| w >= 600.0);
    let italic = has(&["italic", "oblique"]) || t.flags & ITALIC != 0 || t.italic_angle != 0.0;
    family + usize::from(italic) + 2 * usize::from(bold)
}

/// The CJK face that stands in for a CID font: Gothic for the sans
/// serif families by their names, else Mincho.
fn choose_cjk(t: &Traits<'_>) -> usize {
    let name = t
        .base_font
        .map(|n| {
            strip_subset(n)
                .replace([' ', '-', ',', '_'], "")
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let has = |w: &[&str]| w.iter().any(|w| name.contains(w));
    let gothic = has(&[
        "gothic",
        "goth",
        "sans",
        "shingo",
        "futogo",
        "midashigo",
        "kozgo",
        "kakugo",
        "kaku",
        "maru",
        "jun",
        "meiryo",
        "hei",
        "gulim",
        "dotum",
    ]) || (name.contains("go") && !name.contains("min"));
    usize::from(gothic && !has(&["mincho", "ming", "song", "batang"]))
}

fn parsed(cell: &OnceLock<Option<Arc<Program>>>, f: &Face) -> Option<Arc<Program>> {
    cell.get_or_init(|| Program::parse(Cow::Borrowed(f.data)).ok().map(Arc::new))
        .clone()
}

/// The substitute for a simple font: its name and program.
pub fn substitute(t: &Traits<'_>) -> Option<(&'static str, Arc<Program>)> {
    let k = choose(t);
    Some((FACES[k].name, parsed(&PARSED[k], &FACES[k])?))
}

/// The substitute for a CID font: its name and program (CID-keyed, in
/// Adobe-Japan1 order, with a Unicode cmap).
pub fn substitute_cjk(t: &Traits<'_>) -> Option<(&'static str, Arc<Program>)> {
    let k = choose_cjk(t);
    Some((CJK_FACES[k].name, parsed(&CJK_PARSED[k], &CJK_FACES[k])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(n: &str) -> &'static str {
        FACES[choose(&Traits {
            base_font: Some(n),
            ..Traits::default()
        })]
        .name
    }

    #[test]
    fn choices() {
        assert_eq!(named("Helvetica"), "Foxit Sans");
        assert_eq!(named("ABCDEF+Arial,BoldItalic"), "Foxit Sans Bold Italic");
        assert_eq!(named("Times-Roman"), "Foxit Serif");
        assert_eq!(named("Times-BoldItalic"), "Foxit Serif Bold Italic");
        assert_eq!(named("Courier-Oblique"), "Foxit Fixed Italic");
        assert_eq!(named("Symbol"), "Foxit Symbol");
        assert_eq!(named("ZapfDingbats"), "Foxit Dingbats");
        let serif = Traits {
            base_font: Some("Foo"),
            flags: SERIF | ITALIC,
            ..Traits::default()
        };
        assert_eq!(FACES[choose(&serif)].name, "Foxit Serif Italic");
    }

    #[test]
    fn all_faces_parse_and_draw() {
        for (k, f) in FACES.iter().enumerate() {
            let p =
                Program::parse(Cow::Borrowed(f.data)).unwrap_or_else(|e| panic!("{}: {e}", f.name));
            let glyph = match k {
                12 => "alpha",
                13 => "a1",
                _ => "A",
            };
            let o = p
                .outline_by_name(glyph)
                .unwrap_or_else(|| panic!("{}: no {glyph}", f.name));
            let b = o.bbox().unwrap();
            // Glyph space to text space: about an em tall at most.
            assert!(b[3] > 0.3 && b[3] < 1.2, "{}: {b:?}", f.name);
        }
        let p = substitute(&Traits::default()).unwrap().1;
        assert_eq!(p.format, crate::Format::Cff);
        assert_eq!(p.builtin_name(b'A').as_deref(), Some("A"));
        let sym = substitute(&Traits {
            base_font: Some("Symbol"),
            ..Traits::default()
        })
        .unwrap()
        .1;
        assert!(sym.has_glyph_name("alpha"));
    }

    #[test]
    fn cjk_faces() {
        let named = |n: &str| {
            CJK_FACES[choose_cjk(&Traits {
                base_font: Some(n),
                ..Traits::default()
            })]
            .name
        };
        for n in [
            "Ryumin-Light",
            "MidashiMin-MA31",
            "MS-Mincho",
            "KozMinPr6N-Regular",
        ] {
            assert_eq!(named(n), "Harano Aji Mincho", "{n}");
        }
        for n in [
            "GothicBBB-Medium",
            "ABCDEF+ShinGo-Bold",
            "FutoGoB101-Bold",
            "Jun101-Light",
            "MSPGothic",
            "HiraKakuPro-W3",
        ] {
            assert_eq!(named(n), "Harano Aji Gothic", "{n}");
        }
        for f in &CJK_FACES {
            let p = Program::parse(Cow::Borrowed(f.data)).unwrap();
            assert!(p.cid_keyed, "{}", f.name);
            // CID 843 is あ (Adobe-Japan1); U+6F22 is 漢.
            let a = p.gid_of_cid(843).and_then(|g| p.outline(g)).unwrap();
            let b = a.bbox().unwrap();
            assert!(b[3] > 0.3 && b[3] < 1.2, "{}: {b:?}", f.name);
            assert!(
                p.cmap(3, 10, 0x6f22)
                    .or_else(|| p.cmap(3, 1, 0x6f22))
                    .is_some()
            );
            // Vertical forms agree with Adobe's vertical CMap: 、。ー（「
            // in Shift-JIS, horizontal CIDs through the font's vertical
            // alternates are the CIDs 90ms-RKSJ-V gives.
            let h = crate::predefined::cmap("90ms-RKSJ-H").unwrap();
            let v = crate::predefined::cmap("90ms-RKSJ-V").unwrap();
            for code in [0x8141, 0x8142, 0x815b, 0x8169, 0x8175] {
                let gid = |c: Option<u32>| p.gid_of_cid(c.unwrap()).unwrap();
                let (hg, vg) = (gid(h.cid((2, code))), gid(v.cid((2, code))));
                assert_ne!(hg, vg, "{code:x}");
                assert_eq!(p.vertical_gid(hg), vg, "{}: {code:x}", f.name);
                assert_eq!(p.vertical_gid(vg), vg);
            }
            // Kanji have no vertical form.
            let k = p.gid_of_cid(1533).unwrap();
            assert_eq!(p.vertical_gid(k), k);
        }
    }
}
