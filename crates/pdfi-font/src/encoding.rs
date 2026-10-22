//! Simple-font encodings (code → glyph name), glyph names → text (Adobe
//! Glyph List), and the standard 14 fonts: their widths (used when a font
//! dictionary has no `/Widths`) and built-in encodings.

use crate::tables::{GLYPH_LIST, MAC_ROMAN, STANDARD, WIN_ANSI};

/// Glyph names by code.
pub type Table = [&'static str; 256];

/// A base encoding by its name.
pub fn base(name: &[u8]) -> Option<&'static Table> {
    match name {
        b"WinAnsiEncoding" => Some(&WIN_ANSI),
        b"MacRomanEncoding" => Some(&MAC_ROMAN),
        // MacExpertEncoding names small capitals and old-style figures;
        // their text is the standard one.
        b"StandardEncoding" | b"MacExpertEncoding" => Some(&STANDARD),
        _ => None,
    }
}

pub fn standard() -> &'static Table {
    &STANDARD
}

pub fn win_ansi() -> &'static Table {
    &WIN_ANSI
}

/// The `MacRomanEncoding` code of a glyph name (for `(1, 0)` cmaps).
pub fn mac_roman_code(name: &str) -> Option<u8> {
    MAC_ROMAN.iter().position(|&n| n == name).map(|c| c as u8)
}

/// Remove a subset prefix (`ABCDEF+Name` → `Name`).
pub fn strip_subset(s: &str) -> &str {
    match s.split_once('+') {
        Some((p, rest)) if p.len() == 6 && p.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => s,
    }
}

/// The Unicode value of one glyph name component (`A`, `uni0041`,
/// `u1F600`), by the Adobe Glyph List rules.
fn component(n: &str) -> Option<String> {
    if let Ok(k) = GLYPH_LIST.binary_search_by(|e| e.0.cmp(n)) {
        return char::from_u32(GLYPH_LIST[k].1 as u32).map(String::from);
    }
    if let Some(hex) = n.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let units: Vec<u16> = (0..hex.len() / 4)
                .filter_map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok())
                .collect();
            return Some(String::from_utf16_lossy(&units));
        }
    }
    if let Some(hex) = n.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return u32::from_str_radix(hex, 16)
                .ok()
                .and_then(char::from_u32)
                .map(String::from);
        }
    }
    None
}

/// The text a glyph name stands for: Adobe Glyph List names, `uniXXXX`
/// (one or more), `uXXXX[XX]`, suffixes (`a.sc`) and ligatures (`f_i`).
pub fn glyph_text(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base.is_empty() {
        return None;
    }
    let mut out = String::new();
    for part in base.split('_') {
        out.push_str(&component(part)?);
    }
    Some(out)
}

/// The single character of a glyph name, if it is one.
pub fn glyph_char(name: &str) -> Option<char> {
    let t = glyph_text(name)?;
    let mut c = t.chars();
    let first = c.next()?;
    c.next().is_none().then_some(first)
}

/// A standard 14 font (or a common alias of one): glyph widths from
/// Adobe's AFM files, and its built-in encoding.
#[derive(Clone, Copy, Debug)]
pub struct Std14 {
    /// `Helvetica-Bold`, ...
    pub name: &'static str,
    widths: &'static [(&'static str, u16)],
    pub encoding: &'static Table,
    /// Symbol and ZapfDingbats.
    pub symbolic: bool,
}

impl Std14 {
    /// Width of a glyph in thousandths of an em.
    pub fn width(&self, glyph: &str) -> Option<f64> {
        let k = self.widths.binary_search_by(|e| e.0.cmp(glyph)).ok()?;
        Some(f64::from(self.widths[k].1))
    }
}

/// The standard 14 font a `/BaseFont` names (Arial, Courier New and
/// Times New Roman count as Helvetica, Courier and Times).
pub fn std14(base: &str) -> Option<Std14> {
    use crate::std14::*;
    let s = strip_subset(base)
        .replace([' ', '-', ',', '_'], "")
        .to_ascii_lowercase();
    let bold = s.contains("bold");
    let italic = s.contains("italic") || s.contains("oblique");
    let pick = |name, widths: &'static [(&'static str, u16)]| Std14 {
        name,
        widths,
        encoding: &STANDARD,
        symbolic: false,
    };
    Some(if s.starts_with("courier") {
        match (bold, italic) {
            (false, false) => pick("Courier", &COURIER_WIDTHS),
            (true, false) => pick("Courier-Bold", &COURIER_BOLD_WIDTHS),
            (false, true) => pick("Courier-Oblique", &COURIER_OBLIQUE_WIDTHS),
            (true, true) => pick("Courier-BoldOblique", &COURIER_BOLD_OBLIQUE_WIDTHS),
        }
    } else if s.starts_with("helvetica") || s.starts_with("arial") {
        match (bold, italic) {
            (false, false) => pick("Helvetica", &HELVETICA_WIDTHS),
            (true, false) => pick("Helvetica-Bold", &HELVETICA_BOLD_WIDTHS),
            (false, true) => pick("Helvetica-Oblique", &HELVETICA_OBLIQUE_WIDTHS),
            (true, true) => pick("Helvetica-BoldOblique", &HELVETICA_BOLD_OBLIQUE_WIDTHS),
        }
    } else if s.starts_with("times") {
        match (bold, italic) {
            (false, false) => pick("Times-Roman", &TIMES_ROMAN_WIDTHS),
            (true, false) => pick("Times-Bold", &TIMES_BOLD_WIDTHS),
            (false, true) => pick("Times-Italic", &TIMES_ITALIC_WIDTHS),
            (true, true) => pick("Times-BoldItalic", &TIMES_BOLD_ITALIC_WIDTHS),
        }
    } else if s == "symbol" || s == "symbolmt" {
        Std14 {
            name: "Symbol",
            widths: &SYMBOL_WIDTHS,
            encoding: &SYMBOL_ENCODING,
            symbolic: true,
        }
    } else if s == "zapfdingbats" {
        Std14 {
            name: "ZapfDingbats",
            widths: &ZAPF_DING_BATS_WIDTHS,
            encoding: &ZAPF_DINGBATS_ENCODING,
            symbolic: true,
        }
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        assert_eq!(WIN_ANSI[0x80], "Euro");
        assert_eq!(WIN_ANSI[b'A' as usize], "A");
        assert_eq!(MAC_ROMAN[0x80], "Adieresis");
        assert_eq!(STANDARD[0x27], "quoteright");
        assert_eq!(mac_roman_code("Adieresis"), Some(0x80));
        assert!(GLYPH_LIST.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn names() {
        assert_eq!(glyph_char("Aacute"), Some('Á'));
        assert_eq!(glyph_char("ydieresis"), Some('ÿ'));
        assert_eq!(glyph_char("uni3042"), Some('あ'));
        assert_eq!(glyph_char("g.sc"), Some('g'));
        assert_eq!(glyph_char("asciitilde"), Some('~'));
        assert_eq!(glyph_char("alpha"), Some('α'));
        assert_eq!(glyph_char("u1F600"), Some('😀'));
        assert_eq!(glyph_text("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(glyph_text("uni00410042").as_deref(), Some("AB"));
        assert_eq!(glyph_text("g123"), None);
        assert_eq!(glyph_text(".notdef"), None);
    }

    #[test]
    fn standard_14() {
        let h = std14("ABCDEF+Helvetica-Bold").unwrap();
        assert_eq!(h.name, "Helvetica-Bold");
        assert_eq!(h.width("A"), Some(722.0));
        assert_eq!(h.width("Euro"), Some(556.0));
        assert_eq!(std14("Courier-Oblique").unwrap().width("A"), Some(600.0));
        assert_eq!(
            std14("Arial,BoldItalic").unwrap().name,
            "Helvetica-BoldOblique"
        );
        let s = std14("Symbol").unwrap();
        assert!(s.symbolic);
        assert_eq!(s.encoding[b'a' as usize], "alpha");
        assert!(std14("Foo").is_none());
        for w in [
            &crate::std14::TIMES_ROMAN_WIDTHS[..],
            &crate::std14::SYMBOL_WIDTHS[..],
        ] {
            assert!(w.windows(2).all(|p| p[0].0 < p[1].0));
        }
    }
}
