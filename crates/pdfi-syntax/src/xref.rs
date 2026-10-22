//! Cross-reference data: entries, revisions and the parsers for classic
//! tables and xref stream data.

use crate::lexer::{LexError, Lexer, Tok};
use crate::object::{Dict, DictExt, ObjId, Object, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefEntry {
    Free { next: u32, generation: u16 },
    InUse { offset: u64, generation: u16 },
    Compressed { stream: u32, index: u32 },
}

impl XrefEntry {
    pub fn is_live(&self) -> bool {
        !matches!(self, XrefEntry::Free { .. })
    }

    pub fn generation(&self) -> u16 {
        match *self {
            XrefEntry::Free { generation, .. } | XrefEntry::InUse { generation, .. } => generation,
            XrefEntry::Compressed { .. } => 0,
        }
    }
}

/// Entries of a section plus non-fatal warnings.
pub type Entries = (Vec<(u32, XrefEntry)>, Vec<String>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefKind {
    Table,
    Stream,
    /// Classic table plus an `/XRefStm` stream (PDF 1.5 hybrid file).
    Hybrid,
    /// Reconstructed by scanning the file.
    Recovered,
}

/// One cross-reference section (one incremental update).
#[derive(Clone, Debug)]
pub struct Revision {
    pub kind: XrefKind,
    /// Offset the section was found at (the `startxref` / `/Prev` value,
    /// after any adjustment).
    pub offset: u64,
    /// Bytes of the section including the trailer (classic tables) or the
    /// xref stream object.
    pub span: Span,
    pub trailer: Dict,
    pub entries: Vec<(u32, XrefEntry)>,
    /// Entries from the `/XRefStm` stream of a hybrid section.
    pub stream_entries: Vec<(u32, XrefEntry)>,
    pub xref_stream: Option<ObjId>,
}

/// Merged view over all revisions: newest entry per object number.
#[derive(Clone, Debug, Default)]
pub struct XrefTable {
    entries: Vec<Option<XrefEntry>>,
}

impl XrefTable {
    pub fn get(&self, num: u32) -> Option<XrefEntry> {
        self.entries.get(num as usize).copied().flatten()
    }

    pub fn set(&mut self, num: u32, e: XrefEntry) {
        let i = num as usize;
        if i >= self.entries.len() {
            self.entries.resize(i + 1, None);
        }
        self.entries[i] = Some(e);
    }

    /// Number of slots (highest object number + 1).
    pub fn size(&self) -> u32 {
        self.entries.len() as u32
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, XrefEntry)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|e| (i as u32, e)))
    }

    pub fn live_ids(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.iter()
            .filter(|(_, e)| e.is_live())
            .map(|(n, e)| ObjId::new(n, e.generation()))
    }

    /// Merge revisions given newest first.
    pub fn from_revisions(revs: &[Revision]) -> Self {
        let mut t = XrefTable::default();
        for rev in revs.iter().rev() {
            let in_stream: std::collections::HashSet<u32> = rev
                .stream_entries
                .iter()
                .filter(|(_, e)| e.is_live())
                .map(|&(n, _)| n)
                .collect();
            for &(n, e) in &rev.stream_entries {
                t.set(n, e);
            }
            for &(n, e) in &rev.entries {
                // In hybrid sections, free table entries do not hide objects
                // found in the stream part.
                if e.is_live() || !in_stream.contains(&n) {
                    t.set(n, e);
                }
            }
        }
        t
    }
}

/// Parse a classic table body after the `xref` keyword up to (not
/// including) `trailer`. Returns the entries and the position of `trailer`.
///
/// `Err(Eof)` means more data is needed.
pub fn parse_table(lex: &mut Lexer<'_>) -> Result<Entries, LexError> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    loop {
        let save = lex.pos();
        let t = lex.next_token()?;
        let start = match t.tok {
            Tok::Int(s) if s >= 0 => s as u64,
            Tok::Keyword(b"trailer") => {
                lex.set_pos(save);
                break;
            }
            _ => {
                warnings.push(format!(
                    "unexpected token in xref table at {}",
                    t.span.start
                ));
                lex.set_pos(save);
                break;
            }
        };
        let count = match lex.next_token()?.tok {
            Tok::Int(c) if c >= 0 => c as u64,
            _ => {
                warnings.push("invalid xref subsection header".into());
                break;
            }
        };
        let first_index = entries.len();
        for i in 0..count {
            let off = lex.next_token()?;
            let generation = lex.next_token()?;
            let kind = lex.next_token()?;
            let (Tok::Int(off), Tok::Int(g)) = (off.tok, generation.tok) else {
                warnings.push("malformed xref entry".into());
                return Ok((entries, warnings));
            };
            let num = (start + i) as u32;
            let generation = g.clamp(0, i64::from(u16::MAX)) as u16;
            let e = match kind.tok {
                Tok::Keyword(b"n") => XrefEntry::InUse {
                    offset: off.max(0) as u64,
                    generation,
                },
                Tok::Keyword(b"f") => XrefEntry::Free {
                    next: off.max(0) as u32,
                    generation,
                },
                _ => {
                    warnings.push(format!("xref entry for object {num} has invalid type"));
                    XrefEntry::Free {
                        next: 0,
                        generation,
                    }
                }
            };
            entries.push((num, e));
        }
        // Common corruption: a section starting at 1 whose first entry is the
        // free head of the list (object 0).
        if start == 1
            && matches!(
                entries.get(first_index),
                Some((
                    _,
                    XrefEntry::Free {
                        generation: 65535,
                        ..
                    }
                ))
            )
        {
            warnings.push("xref subsection numbered from 1 instead of 0; renumbered".into());
            for e in &mut entries[first_index..] {
                e.0 -= 1;
            }
        }
    }
    Ok((entries, warnings))
}

/// Decode xref stream rows (already filter-decoded) per `/W` and `/Index`.
pub fn parse_stream_entries(dict: &Dict, data: &[u8]) -> Result<Entries, String> {
    let mut warnings = Vec::new();
    let w: Vec<usize> = match dict.get_key("W") {
        Some(Object::Array(a)) if a.len() == 3 => a
            .iter()
            .map(|n| n.obj.as_i64().unwrap_or(0).clamp(0, 8) as usize)
            .collect(),
        _ => return Err("xref stream without valid /W".into()),
    };
    let row = w[0] + w[1] + w[2];
    if row == 0 {
        return Err("xref stream /W is all zero".into());
    }
    let size = dict.get_i64("Size").unwrap_or(0).max(0) as u64;
    let index: Vec<(u64, u64)> = match dict.get_key("Index") {
        Some(Object::Array(a)) => a
            .chunks(2)
            .filter_map(|c| match c {
                [s, n] => Some((s.obj.as_i64()?.max(0) as u64, n.obj.as_i64()?.max(0) as u64)),
                _ => None,
            })
            .collect(),
        _ => vec![(0, size)],
    };
    let field = |bytes: &[u8]| bytes.iter().fold(0u64, |acc, &b| acc << 8 | u64::from(b));
    let mut entries = Vec::new();
    let mut rows = data.chunks_exact(row);
    'outer: for (start, count) in index {
        for i in 0..count {
            let Some(r) = rows.next() else {
                warnings.push("xref stream data shorter than /Index requires".into());
                break 'outer;
            };
            let t = if w[0] == 0 { 1 } else { field(&r[..w[0]]) };
            let f2 = field(&r[w[0]..w[0] + w[1]]);
            let f3 = field(&r[w[0] + w[1]..]);
            let num = (start + i) as u32;
            let e = match t {
                0 => XrefEntry::Free {
                    next: f2 as u32,
                    generation: f3.min(65535) as u16,
                },
                1 => XrefEntry::InUse {
                    offset: f2,
                    generation: f3.min(65535) as u16,
                },
                2 => XrefEntry::Compressed {
                    stream: f2 as u32,
                    index: f3 as u32,
                },
                // Unknown types are to be treated as null references.
                _ => continue,
            };
            entries.push((num, e));
        }
    }
    Ok((entries, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Name, Node};

    #[test]
    fn classic_table() {
        let src = b"0 3\n0000000000 65535 f\r\n0000000017 00000 n\r\n0000000081 00000 n\r\n5 1\n0000000200 00002 n \ntrailer";
        let mut l = Lexer::new_complete(src, 0);
        let (e, w) = parse_table(&mut l).unwrap();
        assert!(w.is_empty());
        assert_eq!(e.len(), 4);
        assert_eq!(
            e[1],
            (
                1,
                XrefEntry::InUse {
                    offset: 17,
                    generation: 0
                }
            )
        );
        assert_eq!(
            e[3],
            (
                5,
                XrefEntry::InUse {
                    offset: 200,
                    generation: 2
                }
            )
        );
        assert_eq!(src[l.pos()..].trim_ascii_start(), b"trailer");
    }

    #[test]
    fn off_by_one_table() {
        let src = b"1 2\n0000000000 65535 f \n0000000017 00000 n \ntrailer";
        let (e, w) = parse_table(&mut Lexer::new_complete(src, 0)).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(
            e[1],
            (
                1,
                XrefEntry::InUse {
                    offset: 17,
                    generation: 0
                }
            )
        );
    }

    #[test]
    fn stream_rows() {
        let mut d = Dict::new();
        let arr = |v: &[i64]| {
            Node::synthetic(Object::Array(
                v.iter()
                    .map(|&i| Node::synthetic(Object::Integer(i)))
                    .collect(),
            ))
        };
        d.insert(Name::from("W"), arr(&[1, 2, 1]));
        d.insert(Name::from("Index"), arr(&[10, 2]));
        let data = [1, 0x01, 0x00, 0, 2, 0x00, 0x07, 3];
        let (e, _) = parse_stream_entries(&d, &data).unwrap();
        assert_eq!(
            e,
            vec![
                (
                    10,
                    XrefEntry::InUse {
                        offset: 256,
                        generation: 0
                    }
                ),
                (
                    11,
                    XrefEntry::Compressed {
                        stream: 7,
                        index: 3
                    }
                ),
            ]
        );
    }
}
