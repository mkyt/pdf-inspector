//! Read arbitrary bytes as a font program (TrueType, OpenType, CFF or
//! Type 1, told from the data) and outline its glyphs.
#![no_main]

use std::borrow::Cow;

use libfuzzer_sys::fuzz_target;
use pdfi_font::Program;

fuzz_target!(|data: &[u8]| {
    let Ok(p) = Program::parse(Cow::Owned(data.to_vec())) else {
        return;
    };
    for gid in 0..p.glyphs.unwrap_or(0).min(64) as u16 {
        let _ = p.outline(gid);
        let _ = p.glyph_name(gid);
        let _ = p.advance(gid);
    }
    for code in [0u8, 32, 65, 97, 255] {
        let _ = p.builtin_name(code);
        let _ = p.builtin_gid(code);
        let _ = p.cmap(3, 1, code.into());
        let _ = p.cmap(3, 0, 0xf000 | u32::from(code));
    }
    let _ = p.outline_by_name("A");
    let _ = p.gid_of_cid(1);
});
