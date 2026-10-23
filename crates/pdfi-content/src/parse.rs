//! Content stream parser: decoded stream data to [`Op`]s.
//!
//! The streams of a page's `/Contents` are parsed as one sequence (operands
//! may precede their operator in an earlier stream). Problems are reported
//! and parsing goes on: unknown operators are kept, stray tokens skipped.

use pdfi_syntax::lexer::{Tok, is_whitespace};
use pdfi_syntax::parser::{ParseError, Parser};
use pdfi_syntax::{Dict, DictExt, Name, Node, Object, Span};

use crate::op::{ContentProgram, ContentSource, InlineImage, Op, OpOrigin, Operator};
use crate::{Issue, Issues};

/// Operations kept from one content.
pub const MAX_OPS: usize = 4_000_000;
/// Operands kept for one operator; more are dropped.
const MAX_OPERANDS: usize = 1000;
/// Bytes after `EI` that must look like content for a scanned `EI` to count.
const EI_LOOKAHEAD: usize = 32;

/// Decoded data of one stream of the content.
#[derive(Debug)]
pub struct SourceData {
    pub id: Option<pdfi_syntax::ObjId>,
    pub data: Vec<u8>,
    /// Decoding failed or stopped early.
    pub error: Option<String>,
}

/// Where reading can resume: the reader's state between two operations.
#[derive(Clone, Debug, Default)]
pub struct ReadPoint {
    source: usize,
    offset: usize,
    /// Operands read but not yet used (only when an operator's operands
    /// are in an earlier stream).
    stack: Vec<Node>,
    stack_source: u32,
    count: usize,
}

/// Reads the operations of a content one at a time, so that a content of
/// millions of operations need not be held as [`Op`]s.
pub struct OpReader<'a> {
    sources: &'a [SourceData],
    si: usize,
    p: Parser<'a>,
    stack: Vec<Node>,
    stack_source: u32,
    dropped: bool,
    count: usize,
    /// The current source's decoding problem has been reported.
    entered: bool,
    /// Stopped at [`MAX_OPS`].
    pub truncated: bool,
    finished: bool,
    /// The last operation's operands began in an earlier stream.
    pub(crate) carried: bool,
}

impl<'a> OpReader<'a> {
    pub fn new(sources: &'a [SourceData]) -> Self {
        Self::at(sources, &ReadPoint::default())
    }

    /// Resume at `rp` (taken with [`OpReader::point`] on the same sources).
    pub fn at(sources: &'a [SourceData], rp: &ReadPoint) -> Self {
        let data: &[u8] = sources.get(rp.source).map_or(&[], |s| &s.data);
        let mut p = Parser::new(data, 0, true);
        p.lex.set_pos(rp.offset);
        OpReader {
            sources,
            si: rp.source,
            p,
            stack: rp.stack.clone(),
            stack_source: rp.stack_source,
            dropped: false,
            count: rp.count,
            entered: rp.offset > 0,
            truncated: false,
            finished: false,
            carried: false,
        }
    }

    pub fn point(&self) -> ReadPoint {
        ReadPoint {
            source: self.si,
            offset: self.p.lex.pos(),
            stack: self.stack.clone(),
            stack_source: self.stack_source,
            count: self.count,
        }
    }

    /// Operations read so far (the index of the next one).
    pub fn count(&self) -> usize {
        self.count
    }

    fn emit(&mut self, issues: &mut Issues, op: Op) -> Option<Op> {
        issues.flush_lexer(&mut self.p, self.si as u32, self.count);
        self.count += 1;
        Some(op)
    }

    /// The next operation; problems go to `issues`.
    pub fn next(&mut self, issues: &mut Issues) -> Option<Op> {
        loop {
            if self.finished {
                return None;
            }
            let Some(s) = self.sources.get(self.si) else {
                self.finished = true;
                self.leftover(issues);
                return None;
            };
            let si = self.si as u32;
            if !self.entered {
                self.entered = true;
                if let Some(e) = &s.error {
                    issues.push(Issue::error(
                        "StreamDecode",
                        format!("content stream {}: {e}", source_name(s, si)),
                        None,
                    ));
                }
            }
            if self.count >= MAX_OPS {
                self.truncated = true;
                self.finished = true;
                issues.push(Issue::error(
                    "TooManyOps",
                    format!("only the first {MAX_OPS} operations are read"),
                    None,
                ));
                return None;
            }
            // Problems found from here on belong to the next operation.
            issues.cursor = Some(self.count as u32);
            let before = self.p.lex.pos();
            let t = match self.p.next_tok() {
                Ok(t) => t,
                Err(_) => {
                    // Report the unfinished token from its start.
                    self.p.lex.set_pos(before);
                    self.end_of_source(issues);
                    continue;
                }
            };
            let span = t.span;
            let node = match t.tok {
                Tok::Int(i) => Some(Node::new(Object::Integer(i), span)),
                Tok::Real(r) => Some(Node::new(Object::Real(r), span)),
                Tok::Name(n) => Some(Node::new(Object::Name(Name(n)), span)),
                Tok::Str(s) => Some(Node::new(Object::String(s), span)),
                Tok::ArrayOpen | Tok::DictOpen => match self.p.object_from(t) {
                    Ok(n) => Some(n),
                    Err(ParseError::Eof) => {
                        issues.push(
                            Issue::error("Syntax", "array or dictionary not closed", None)
                                .at(si, Span::new(span.start, s.data.len() as u64)),
                        );
                        self.p.lex.set_pos(s.data.len());
                        None
                    }
                    Err(ParseError::Syntax { message, span }) => {
                        issues.push(Issue::error("Syntax", message, None).at(si, span));
                        None
                    }
                },
                Tok::Keyword(b"true") => Some(Node::new(Object::Bool(true), span)),
                Tok::Keyword(b"false") => Some(Node::new(Object::Bool(false), span)),
                Tok::Keyword(b"null") => Some(Node::new(Object::Null, span)),
                Tok::Keyword(b"BI") => {
                    self.carried = !self.stack.is_empty() && self.stack_source != si;
                    let start = self
                        .stack
                        .first()
                        .filter(|_| self.stack_source == si)
                        .and_then(|n| n.span)
                        .map_or(span.start, |s| s.start);
                    if !self.stack.is_empty() {
                        issues.push(
                            Issue::warning("Operands", "operands before BI are ignored", None)
                                .at(si, span),
                        );
                        self.stack.clear();
                    }
                    let (img, end) = inline_image(&mut self.p, si, issues);
                    let op = Op {
                        operator: Operator::InlineImage,
                        operands: Vec::new(),
                        origin: Some(OpOrigin {
                            source: si,
                            span: Span::new(start, end),
                        }),
                        inline_image: Some(Box::new(img)),
                    };
                    return self.emit(issues, op);
                }
                Tok::Keyword(k) if is_operator_like(k) => {
                    let operator = Operator::from_bytes(k);
                    self.carried = !self.stack.is_empty() && self.stack_source != si;
                    let start = self
                        .stack
                        .first()
                        .filter(|_| self.stack_source == si)
                        .and_then(|n| n.span)
                        .map_or(span.start, |s| s.start);
                    self.dropped = false;
                    let op = Op {
                        operator,
                        operands: std::mem::take(&mut self.stack),
                        origin: Some(OpOrigin {
                            source: si,
                            span: Span::new(start, span.end),
                        }),
                        inline_image: None,
                    };
                    return self.emit(issues, op);
                }
                Tok::Keyword(k) => {
                    issues.push(
                        Issue::error(
                            "Syntax",
                            format!(
                                "unexpected '{}'",
                                String::from_utf8_lossy(&k[..k.len().min(32)])
                            ),
                            None,
                        )
                        .at(si, span),
                    );
                    None
                }
                Tok::ArrayClose | Tok::DictClose | Tok::BraceOpen | Tok::BraceClose => {
                    issues.push(Issue::error("Syntax", "unexpected delimiter", None).at(si, span));
                    None
                }
            };
            if let Some(n) = node {
                if self.stack.is_empty() {
                    self.stack_source = si;
                }
                if self.stack.len() < MAX_OPERANDS {
                    self.stack.push(n);
                } else if !self.dropped {
                    self.dropped = true;
                    issues.push(
                        Issue::error(
                            "Operands",
                            format!(
                                "more than {MAX_OPERANDS} operands in a row; the rest are dropped"
                            ),
                            None,
                        )
                        .at(si, span),
                    );
                }
            }
        }
    }

    /// The current source is used up: report a truncated token and go on
    /// with the next one (operands carry over).
    fn end_of_source(&mut self, issues: &mut Issues) {
        let si = self.si as u32;
        let len = self.sources[self.si].data.len();
        self.p.lex.skip_ws();
        if !self.p.lex.at_end() {
            let at = self.p.lex.pos() as u64;
            issues.push(
                Issue::error(
                    "Syntax",
                    "unterminated token at the end of the stream",
                    None,
                )
                .at(si, Span::new(at, len as u64)),
            );
        }
        issues.flush_lexer(&mut self.p, si, self.count);
        self.si += 1;
        self.entered = false;
        let data: &[u8] = self.sources.get(self.si).map_or(&[], |s| &s.data);
        self.p = Parser::new(data, 0, true);
    }

    fn leftover(&mut self, issues: &mut Issues) {
        if let Some(first) = self.stack.first() {
            let end = self.stack.last().and_then(|n| n.span).map_or(0, |s| s.end);
            issues.push(
                Issue::warning(
                    "Operands",
                    format!(
                        "{} operand(s) at the end without an operator",
                        self.stack.len()
                    ),
                    None,
                )
                .at(
                    self.stack_source,
                    Span::new(first.span.map_or(0, |s| s.start), end),
                ),
            );
        }
    }
}

/// Parse `sources` as one content, keeping every operation.
pub fn parse(sources: &[SourceData], issues: &mut Issues) -> ContentProgram {
    let mut r = OpReader::new(sources);
    let mut ops = Vec::new();
    while let Some(op) = r.next(issues) {
        ops.push(op);
    }
    ContentProgram {
        sources: sources
            .iter()
            .map(|s| ContentSource {
                id: s.id,
                len: s.data.len() as u64,
                error: s.error.clone(),
            })
            .collect(),
        ops,
        truncated: r.truncated,
    }
}

/// Could be an operator (known or not), rather than garbage.
fn is_operator_like(k: &[u8]) -> bool {
    k.len() <= 16
        && k.iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'*' | b'\'' | b'"'))
}

fn source_name(s: &SourceData, i: u32) -> String {
    match s.id {
        Some(id) => format!("[{i}] {id} obj"),
        None => format!("[{i}]"),
    }
}

/// Look up an inline image entry by its full or abbreviated key.
fn entry<'a>(d: &'a Dict, long: &str, short: &str) -> Option<&'a Object> {
    d.get_key(long).or_else(|| d.get_key(short))
}

/// Read `<< ... >> ID data EI` after `BI`. Returns the image and the end of
/// `EI`.
fn inline_image(p: &mut Parser<'_>, si: u32, issues: &mut Issues) -> (InlineImage, u64) {
    let mut dict = Dict::new();
    let buf = p.lex.buf();
    loop {
        let t = match p.next_tok() {
            Ok(t) => t,
            Err(_) => {
                issues.push(
                    Issue::error("InlineImage", "BI without ID", None)
                        .at(si, Span::new(p.lex.pos() as u64, p.lex.pos() as u64)),
                );
                let end = buf.len() as u64;
                return (
                    InlineImage {
                        dict,
                        data: Span::new(end, end),
                    },
                    end,
                );
            }
        };
        match t.tok {
            Tok::Keyword(b"ID") => break,
            Tok::Name(k) => match p.parse_object() {
                Ok(v) => {
                    dict.insert(Name(k), v);
                }
                Err(_) => {
                    issues.push(
                        Issue::error("InlineImage", "inline image entry without a value", None)
                            .at(si, t.span),
                    );
                }
            },
            Tok::Keyword(b"EI") => {
                issues.push(Issue::error("InlineImage", "EI before ID", None).at(si, t.span));
                return (
                    InlineImage {
                        dict,
                        data: Span::new(t.span.start, t.span.start),
                    },
                    t.span.end,
                );
            }
            _ => issues.push(
                Issue::error("InlineImage", "inline image key is not a name", None).at(si, t.span),
            ),
        }
    }
    // A single white-space character separates ID from the data.
    let mut start = p.lex.pos();
    if buf.get(start).copied().is_some_and(is_whitespace) {
        start += 1;
    }
    let found = expected_len(&dict)
        .and_then(|len| {
            ei_after(buf, start.saturating_add(len)).map(|(ei, end)| (start + len, ei, end))
        })
        .or_else(|| scan_ei(buf, start));
    match found {
        Some((data_end, _, end)) => {
            p.lex.set_pos(end);
            (
                InlineImage {
                    dict,
                    data: Span::new(start as u64, data_end as u64),
                },
                end as u64,
            )
        }
        None => {
            issues.push(
                Issue::error("InlineImage", "no EI after the inline image data", None)
                    .at(si, Span::new(start as u64, buf.len() as u64)),
            );
            p.lex.set_pos(buf.len());
            (
                InlineImage {
                    dict,
                    data: Span::new(start as u64, buf.len() as u64),
                },
                buf.len() as u64,
            )
        }
    }
}

/// Length of the data: `/L` (`/Length`), or computed for unfiltered images.
fn expected_len(d: &Dict) -> Option<usize> {
    if let Some(l) = entry(d, "Length", "L").and_then(Object::as_i64) {
        return usize::try_from(l).ok();
    }
    if entry(d, "Filter", "F").is_some_and(|f| !matches!(f, Object::Array(a) if a.is_empty())) {
        return None;
    }
    let w = entry(d, "Width", "W")?.as_i64()?;
    let h = entry(d, "Height", "H")?.as_i64()?;
    let mask = matches!(entry(d, "ImageMask", "IM"), Some(Object::Bool(true)));
    let (comps, bpc) = if mask {
        (1, 1)
    } else {
        let comps = match entry(d, "ColorSpace", "CS")? {
            Object::Name(n) => match n.as_bytes() {
                b"G" | b"DeviceGray" | b"CalGray" => 1,
                b"RGB" | b"DeviceRGB" | b"CalRGB" => 3,
                b"CMYK" | b"DeviceCMYK" => 4,
                b"I" | b"Indexed" => 1,
                _ => return None,
            },
            Object::Array(a) if matches!(a.first().map(|n| &n.obj), Some(Object::Name(n)) if n == "I" || n == "Indexed") => {
                1
            }
            _ => return None,
        };
        (comps, entry(d, "BitsPerComponent", "BPC")?.as_i64()?)
    };
    if !(1..=1 << 20).contains(&w)
        || !(1..=1 << 20).contains(&h)
        || !matches!(bpc, 1 | 2 | 4 | 8 | 16)
    {
        return None;
    }
    let row = (w * comps * bpc + 7) / 8;
    usize::try_from(row.checked_mul(h)?).ok()
}

/// `EI` right after `pos` (white space allowed before it). Returns the start
/// and end of `EI`.
fn ei_after(buf: &[u8], pos: usize) -> Option<(usize, usize)> {
    let mut i = pos;
    while buf.get(i).copied().is_some_and(is_whitespace) {
        i += 1;
    }
    (buf.get(i..i + 2) == Some(b"EI") && ends_token(buf, i + 2)).then_some((i, i + 2))
}

fn ends_token(buf: &[u8], i: usize) -> bool {
    buf.get(i)
        .is_none_or(|&b| is_whitespace(b) || pdfi_syntax::lexer::is_delimiter(b))
}

/// Find ` EI ` followed by what looks like more content.
fn scan_ei(buf: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    let mut i = start;
    while i + 2 <= buf.len() {
        let k = i + buf[i..].windows(2).position(|w| w == b"EI")?;
        let before_ok = k == start || is_whitespace(buf[k - 1]);
        if before_ok && ends_token(buf, k + 2) {
            let tail = &buf[k + 2..buf.len().min(k + 2 + EI_LOOKAHEAD)];
            if tail
                .iter()
                .all(|&b| is_whitespace(b) || (0x21..0x7f).contains(&b))
            {
                let data_end = if k > start { k - 1 } else { k };
                return Some((data_end, k, k + 2));
            }
        }
        i = k + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(src: &[u8]) -> (ContentProgram, Vec<Issue>) {
        let mut issues = Issues::default();
        let p = parse(
            &[SourceData {
                id: None,
                data: src.to_vec(),
                error: None,
            }],
            &mut issues,
        );
        (p, issues.into_vec())
    }

    #[test]
    fn operators_and_operands() {
        let (p, issues) = ops(b"q 1 0 0 1 10 20 cm /F1 12 Tf [(a) -20 (b)] TJ Q");
        assert!(issues.is_empty(), "{issues:?}");
        let names: Vec<_> = p
            .ops
            .iter()
            .map(|o| o.operator.name().into_owned())
            .collect();
        assert_eq!(names, ["q", "cm", "Tf", "TJ", "Q"]);
        assert_eq!(p.ops[1].operands.len(), 6);
        assert_eq!(p.ops[1].origin.unwrap().span, Span::new(2, 18));
        assert_eq!(p.ops[3].operands[0].obj.as_array().unwrap().len(), 3);
    }

    #[test]
    fn concatenated_sources_share_operands() {
        let mut issues = Issues::default();
        let p = parse(
            &[
                SourceData {
                    id: None,
                    data: b"1 0 0".to_vec(),
                    error: None,
                },
                SourceData {
                    id: None,
                    data: b" rg".to_vec(),
                    error: None,
                },
            ],
            &mut issues,
        );
        assert_eq!(p.ops.len(), 1);
        assert_eq!(p.ops[0].operands.len(), 3);
        assert_eq!(p.ops[0].origin.unwrap().source, 1);
    }

    #[test]
    fn inline_image_with_computed_length() {
        let (p, issues) = ops(b"q BI /W 4 /H 1 /CS /G /BPC 8 ID \x00 EI EI Q");
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(p.ops.len(), 3);
        let img = p.ops[1].inline_image.as_ref().unwrap();
        assert_eq!(img.data.len(), 4);
        assert_eq!(p.ops[2].operator, Operator::Restore);
    }

    #[test]
    fn inline_image_scanned() {
        let (p, issues) = ops(b"BI /W 2 /H 2 /CS /RGB /BPC 8 /F /Fl ID xx\x9a\x00 EI\nQ");
        assert!(issues.is_empty(), "{issues:?}");
        let img = p.ops[0].inline_image.as_ref().unwrap();
        assert_eq!(img.data.len(), 4);
        assert_eq!(p.ops[1].operator, Operator::Restore);
    }

    #[test]
    fn resume_from_a_point() {
        let sources = [
            SourceData {
                id: None,
                data: b"q 1 0 0 1 5 5 cm BT /F1 9 Tf 1 0".to_vec(),
                error: None,
            },
            SourceData {
                id: None,
                data: b" Td (x) Tj ET Q".to_vec(),
                error: None,
            },
        ];
        let mut all = Vec::new();
        let mut points = Vec::new();
        let mut r = OpReader::new(&sources);
        loop {
            points.push(r.point());
            match r.next(&mut Issues::default()) {
                Some(op) => all.push(op),
                None => break,
            }
        }
        assert_eq!(all.len(), 8);
        for (k, p) in points.iter().enumerate() {
            let mut again = OpReader::at(&sources, p);
            let mut rest = Vec::new();
            while let Some(op) = again.next(&mut Issues::default()) {
                rest.push(op);
            }
            assert_eq!(rest, all[k..], "from {k}");
        }
    }

    #[test]
    fn junk_is_reported() {
        let (p, issues) = ops(b"1 2 ] foo 3 } ");
        assert_eq!(p.ops.len(), 1);
        assert_eq!(
            p.ops[0].operator,
            Operator::Unknown(b"foo".as_slice().into())
        );
        assert_eq!(issues.len(), 3, "{issues:?}");
    }
}
