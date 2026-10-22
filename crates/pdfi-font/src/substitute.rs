//! Fonts that draw simple fonts which are not embedded: the Foxit fonts
//! that PDFium ships (BSD-3-Clause, see `assets/LICENSE_FOXIT`), metric
//! compatible with the standard 14 fonts. They are bare CFF fonts despite
//! the `.pfb` names, compiled in (265 KB) so the browser build draws the
//! same.

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

/// The substitute for a font: its name and program.
pub fn substitute(t: &Traits<'_>) -> Option<(&'static str, Arc<Program>)> {
    let k = choose(t);
    let p = PARSED[k]
        .get_or_init(|| {
            Program::parse(Cow::Borrowed(FACES[k].data))
                .ok()
                .map(Arc::new)
        })
        .clone()?;
    Some((FACES[k].name, p))
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
}
