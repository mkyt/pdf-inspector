//! Embedded CMaps: `/Encoding` streams of Type 0 fonts (codespace ranges,
//! code → CID) and `/ToUnicode` (code → text).

use std::collections::HashMap;

use pdfi_syntax::lexer::{Lexer, Tok, Token};
use pdfi_syntax::{DictExt, ObjId, Object, PdfFile, Stream};

use crate::encoding::glyph_text;

/// Largest CMap stream read.
const MAX_CMAP: usize = 4 << 20;
/// Codes one `bfrange` / `cidrange` line may cover.
const MAX_RANGE: u32 = 1 << 16;
/// Entries read from a CMap.
pub(crate) const MAX_ENTRIES: usize = 1 << 20;

/// A code as (bytes, value).
pub type CodeKey = (u8, u32);

#[derive(Debug, Default)]
pub struct CMap {
    /// (bytes, low, high).
    pub codespace: Vec<(u8, u32, u32)>,
    pub unicode: HashMap<CodeKey, String>,
    pub cids: HashMap<CodeKey, u32>,
    pub vertical: bool,
    /// `usecmap` names (not followed).
    pub uses: Option<String>,
    /// Stopped at [`MAX_ENTRIES`], or the stream could not be decoded.
    pub problem: Option<String>,
}

fn code_of(b: &[u8]) -> Option<CodeKey> {
    (1..=4).contains(&b.len()).then(|| {
        (
            b.len() as u8,
            b.iter().fold(0u32, |a, &x| a << 8 | u32::from(x)),
        )
    })
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks(2)
        .map(|c| u16::from(c[0]) << 8 | u16::from(*c.get(1).unwrap_or(&0)))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Add `k` to the last UTF-16 unit of `dst` (`bfrange` with a string).
fn bump(dst: &[u8], k: u32) -> String {
    let mut units: Vec<u16> = dst
        .chunks(2)
        .map(|c| u16::from(c[0]) << 8 | u16::from(*c.get(1).unwrap_or(&0)))
        .collect();
    if let Some(last) = units.last_mut() {
        *last = last.wrapping_add(k as u16);
    }
    String::from_utf16_lossy(&units)
}

/// The next token if it is a string.
fn next_str(lex: &mut Lexer<'_>) -> Option<Vec<u8>> {
    match lex.peek() {
        Ok(Token {
            tok: Tok::Str(s), ..
        }) => {
            let _ = lex.next_token();
            Some(s.bytes)
        }
        _ => None,
    }
}

/// Read a CMap stream (`o` is how it was referred to, for decryption).
pub fn read(file: &PdfFile, o: &Object, s: &Stream) -> CMap {
    let id = o.as_ref_id().unwrap_or(ObjId::new(0, 0));
    let d = file.decode_stream(id, s, Some(MAX_CMAP));
    let mut cm = CMap::default();
    if let Some(e) = d.errors.first() {
        cm.problem = Some(e.clone());
    }
    if s.dict.get_key("WMode").and_then(Object::as_i64) == Some(1) {
        cm.vertical = true;
    }
    let mut lex = Lexer::new_complete(&d.data, 0);
    let mut prev: Vec<Tok> = Vec::new();
    let full = |cm: &mut CMap| {
        let over = cm.unicode.len() + cm.cids.len() > MAX_ENTRIES;
        if over {
            cm.problem = Some(format!("only the first {MAX_ENTRIES} entries are read"));
        }
        over
    };
    while let Ok(t) = lex.next_token() {
        let Tok::Keyword(k) = t.tok else {
            prev.push(t.tok);
            if prev.len() > 4 {
                prev.remove(0);
            }
            continue;
        };
        match k {
            b"begincodespacerange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    if let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) {
                        cm.codespace.push((n, l, h));
                    }
                }
            }
            b"beginbfchar" => {
                while let Some(src) = next_str(&mut lex) {
                    let Ok(dst) = lex.next_token() else { break };
                    let Some(code) = code_of(&src) else { continue };
                    match dst.tok {
                        Tok::Str(d) => {
                            cm.unicode.insert(code, utf16(&d.bytes));
                        }
                        Tok::Name(n) => {
                            if let Some(t) = std::str::from_utf8(&n).ok().and_then(glyph_text) {
                                cm.unicode.insert(code, t);
                            }
                        }
                        _ => break,
                    }
                    if full(&mut cm) {
                        break;
                    }
                }
            }
            b"beginbfrange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) else {
                        break;
                    };
                    let h = h.min(l.saturating_add(MAX_RANGE));
                    let Ok(dst) = lex.next_token() else { break };
                    match dst.tok {
                        Tok::Str(d) => {
                            for (k, c) in (l..=h).enumerate() {
                                cm.unicode.insert((n, c), bump(&d.bytes, k as u32));
                            }
                        }
                        Tok::ArrayOpen => {
                            let mut c = l;
                            while let Some(d) = next_str(&mut lex) {
                                if c <= h {
                                    cm.unicode.insert((n, c), utf16(&d));
                                }
                                c = c.saturating_add(1);
                            }
                            // The closing bracket.
                            let _ = lex.next_token();
                        }
                        _ => break,
                    }
                    if full(&mut cm) {
                        break;
                    }
                }
            }
            b"begincidrange" => {
                while let (Some(lo), Some(hi)) = (next_str(&mut lex), next_str(&mut lex)) {
                    let Ok(Token {
                        tok: Tok::Int(cid), ..
                    }) = lex.next_token()
                    else {
                        break;
                    };
                    let (Some((n, l)), Some((_, h))) = (code_of(&lo), code_of(&hi)) else {
                        break;
                    };
                    let h = h.min(l.saturating_add(MAX_RANGE));
                    for (k, code) in (l..=h).enumerate() {
                        cm.cids
                            .insert((n, code), (cid.max(0) as u32).saturating_add(k as u32));
                    }
                    if full(&mut cm) {
                        break;
                    }
                }
            }
            b"begincidchar" => {
                while let Some(src) = next_str(&mut lex) {
                    let Ok(Token {
                        tok: Tok::Int(cid), ..
                    }) = lex.next_token()
                    else {
                        break;
                    };
                    if let Some(code) = code_of(&src) {
                        cm.cids.insert(code, cid.max(0) as u32);
                    }
                    if full(&mut cm) {
                        break;
                    }
                }
            }
            b"def" => {
                if let [.., Tok::Name(n), Tok::Int(1)] = prev.as_slice() {
                    if n == b"WMode" {
                        cm.vertical = true;
                    }
                }
            }
            b"usecmap" => {
                if let [.., Tok::Name(n)] = prev.as_slice() {
                    cm.uses = Some(String::from_utf8_lossy(n).into_owned());
                }
            }
            _ => {}
        }
        prev.clear();
    }
    cm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(code_of(b"\x01\x02"), Some((2, 0x0102)));
        assert_eq!(code_of(b""), None);
        assert_eq!(utf16(b"\x00A\x00B"), "AB");
        assert_eq!(bump(b"\x00A", 2), "C");
    }
}
