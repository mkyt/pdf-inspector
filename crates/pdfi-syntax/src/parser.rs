//! Object parser on top of the [`Lexer`].

use crate::diag::{DiagCode, Diagnostic};
use crate::lexer::{LexError, Lexer, Tok, Token};
use crate::object::{Dict, Name, Node, ObjId, Object, Span};

pub const MAX_DEPTH: u32 = 256;

#[derive(Clone, Debug, PartialEq)]
pub enum ParseError {
    /// The buffer ended before the object did; retry with more data.
    Eof,
    Syntax {
        message: String,
        span: Span,
    },
}

impl From<LexError> for ParseError {
    fn from(_: LexError) -> Self {
        ParseError::Eof
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Eof => write!(f, "unexpected end of data"),
            ParseError::Syntax { message, span } => write!(f, "{message} at offset {}", span.start),
        }
    }
}

/// Result of parsing `N G obj ... (endobj | stream)`.
#[derive(Debug)]
pub struct ParsedIndirect {
    pub id: ObjId,
    pub node: Node,
    pub start: u64,
    /// End of `endobj` (or of the object when `endobj` is missing). For
    /// streams this is the start of the data.
    pub end: u64,
    /// Absolute offset of the first data byte, for streams.
    pub stream_data_start: Option<u64>,
}

pub struct Parser<'a> {
    pub lex: Lexer<'a>,
    depth: u32,
}

impl<'a> Parser<'a> {
    pub fn new(buf: &'a [u8], base: u64, complete: bool) -> Self {
        // `complete`: the buffer holds all available data, so reaching its
        // end is not truncation (content streams, object streams, end of file).
        let lex = if complete {
            Lexer::new_complete(buf, base)
        } else {
            Lexer::new(buf, base)
        };
        Self { lex, depth: 0 }
    }

    pub fn take_diags(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.lex.diags)
    }

    fn diag(&mut self, code: DiagCode, msg: impl Into<String>, span: Span) {
        self.lex
            .diags
            .push(Diagnostic::warning(code, msg).with_span(span));
    }

    pub fn next_tok(&mut self) -> Result<Token<'a>, ParseError> {
        Ok(self.lex.next_token()?)
    }

    pub fn peek(&self) -> Result<Token<'a>, ParseError> {
        Ok(self.lex.peek()?)
    }

    pub fn parse_object(&mut self) -> Result<Node, ParseError> {
        let t = self.next_tok()?;
        self.object_from(t)
    }

    pub fn object_from(&mut self, t: Token<'a>) -> Result<Node, ParseError> {
        let span = t.span;
        let obj = match t.tok {
            Tok::Int(n) => return self.int_or_ref(n, span),
            Tok::Real(r) => Object::Real(r),
            Tok::Name(n) => Object::Name(Name(n)),
            Tok::Str(s) => Object::String(s),
            Tok::ArrayOpen => return self.array(span.start),
            Tok::DictOpen => return self.dict(span.start),
            Tok::Keyword(b"true") => Object::Bool(true),
            Tok::Keyword(b"false") => Object::Bool(false),
            Tok::Keyword(b"null") => Object::Null,
            Tok::Keyword(k) => {
                return Err(ParseError::Syntax {
                    message: format!("unexpected keyword '{}'", String::from_utf8_lossy(k)),
                    span,
                });
            }
            other => {
                return Err(ParseError::Syntax {
                    message: format!("unexpected token {other:?}"),
                    span,
                });
            }
        };
        Ok(Node::new(obj, span))
    }

    fn int_or_ref(&mut self, n: i64, span: Span) -> Result<Node, ParseError> {
        let save = self.lex.pos();
        let int = Node::new(Object::Integer(n), span);
        let g = match self.next_tok() {
            Ok(Token {
                tok: Tok::Int(g), ..
            }) => g,
            Ok(_) => {
                self.lex.set_pos(save);
                return Ok(int);
            }
            Err(e) => {
                self.lex.set_pos(save);
                return if self.lex.is_complete() {
                    Ok(int)
                } else {
                    Err(e)
                };
            }
        };
        match self.next_tok() {
            Ok(Token {
                tok: Tok::Keyword(b"R"),
                span: rspan,
            }) => {
                if let (Ok(num), Ok(generation)) = (u32::try_from(n), u16::try_from(g)) {
                    return Ok(Node::new(
                        Object::Ref(ObjId::new(num, generation)),
                        Span::new(span.start, rspan.end),
                    ));
                }
                self.diag(DiagCode::SyntaxError, "reference number out of range", span);
                Ok(Node::new(Object::Null, Span::new(span.start, rspan.end)))
            }
            Ok(_) => {
                self.lex.set_pos(save);
                Ok(int)
            }
            Err(e) => {
                self.lex.set_pos(save);
                if self.lex.is_complete() {
                    Ok(int)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn enter(&mut self, start: u64) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError::Syntax {
                message: "nesting too deep".into(),
                span: Span::new(start, self.lex.abs_pos()),
            });
        }
        Ok(())
    }

    fn array(&mut self, start: u64) -> Result<Node, ParseError> {
        self.enter(start)?;
        let mut items = Vec::new();
        loop {
            let t = self.peek()?;
            match t.tok {
                Tok::ArrayClose => {
                    self.next_tok()?;
                    break;
                }
                Tok::Keyword(b"endobj" | b"stream" | b"endstream") | Tok::DictClose => {
                    self.diag(DiagCode::UnbalancedDelimiter, "array not closed", t.span);
                    break;
                }
                _ => {
                    let t = self.next_tok()?;
                    let tspan = t.span;
                    match self.object_from(t) {
                        Ok(n) => items.push(n),
                        Err(ParseError::Eof) => return Err(ParseError::Eof),
                        Err(ParseError::Syntax { message, .. }) => {
                            self.diag(DiagCode::UnexpectedToken, message, tspan);
                        }
                    }
                }
            }
        }
        self.depth -= 1;
        Ok(Node::new(
            Object::Array(items),
            Span::new(start, self.lex.abs_pos()),
        ))
    }

    pub fn dict(&mut self, start: u64) -> Result<Node, ParseError> {
        let d = self.dict_body(start)?;
        Ok(Node::new(
            Object::Dict(d),
            Span::new(start, self.lex.abs_pos()),
        ))
    }

    /// Parse dictionary entries after `<<` up to and including `>>`.
    pub fn dict_body(&mut self, start: u64) -> Result<Dict, ParseError> {
        self.enter(start)?;
        let mut d = Dict::new();
        loop {
            let t = self.next_tok()?;
            let key = match t.tok {
                Tok::DictClose => break,
                Tok::Name(n) => Name(n),
                Tok::Keyword(b"endobj" | b"stream" | b"endstream") => {
                    self.diag(
                        DiagCode::UnbalancedDelimiter,
                        "dictionary not closed",
                        t.span,
                    );
                    self.lex.set_pos((t.span.start - self.lex.base()) as usize);
                    break;
                }
                _ => {
                    self.diag(
                        DiagCode::UnexpectedToken,
                        "dictionary key is not a name",
                        t.span,
                    );
                    continue;
                }
            };
            let vt = self.peek()?;
            let value = match vt.tok {
                Tok::DictClose | Tok::Keyword(b"endobj" | b"stream" | b"endstream") => {
                    self.diag(
                        DiagCode::SyntaxError,
                        "dictionary key without value",
                        t.span,
                    );
                    Node::new(Object::Null, t.span)
                }
                _ => {
                    let vt = self.next_tok()?;
                    let vspan = vt.span;
                    match self.object_from(vt) {
                        Ok(n) => n,
                        Err(ParseError::Eof) => return Err(ParseError::Eof),
                        Err(ParseError::Syntax { message, .. }) => {
                            self.diag(DiagCode::UnexpectedToken, message, vspan);
                            Node::new(Object::Null, vspan)
                        }
                    }
                }
            };
            if d.contains_key(&key) {
                self.diag(
                    DiagCode::SyntaxError,
                    format!("duplicate key {key:?}"),
                    t.span,
                );
            }
            d.insert(key, value);
        }
        self.depth -= 1;
        Ok(d)
    }

    /// Parse `N G obj`.
    pub fn indirect_header(&mut self) -> Result<(ObjId, u64), ParseError> {
        let t1 = self.next_tok()?;
        let t2 = self.next_tok()?;
        let t3 = self.next_tok()?;
        match (t1.tok, t2.tok, t3.tok) {
            (Tok::Int(n), Tok::Int(g), Tok::Keyword(b"obj")) => {
                let (Ok(num), Ok(generation)) = (u32::try_from(n), u16::try_from(g)) else {
                    return Err(ParseError::Syntax {
                        message: "object number out of range".into(),
                        span: t1.span,
                    });
                };
                Ok((ObjId::new(num, generation), t1.span.start))
            }
            _ => Err(ParseError::Syntax {
                message: "expected 'N G obj'".into(),
                span: Span::new(t1.span.start, t3.span.end),
            }),
        }
    }

    pub fn parse_indirect(&mut self) -> Result<ParsedIndirect, ParseError> {
        let (id, start) = self.indirect_header()?;
        // An empty object body ("1 0 obj endobj") is read as null.
        let node = match self.peek()? {
            Token {
                tok: Tok::Keyword(b"endobj"),
                span,
            } => {
                self.diag(DiagCode::SyntaxError, "empty object", span);
                Node::new(Object::Null, Span::new(span.start, span.start))
            }
            _ => self.parse_object()?,
        };
        match self.peek() {
            Ok(Token {
                tok: Tok::Keyword(b"endobj"),
                span,
            }) => {
                self.next_tok()?;
                Ok(ParsedIndirect {
                    id,
                    node,
                    start,
                    end: span.end,
                    stream_data_start: None,
                })
            }
            Ok(Token {
                tok: Tok::Keyword(b"stream"),
                span,
            }) => {
                self.next_tok()?;
                if !matches!(node.obj, Object::Dict(_)) {
                    return Err(ParseError::Syntax {
                        message: "stream without dictionary".into(),
                        span,
                    });
                }
                let data_start = self.stream_data_start(span)?;
                Ok(ParsedIndirect {
                    id,
                    node,
                    start,
                    end: data_start,
                    stream_data_start: Some(data_start),
                })
            }
            Ok(t) => {
                self.diag(DiagCode::MissingEndobj, "missing 'endobj'", t.span);
                let end = node.span.map_or(start, |s| s.end);
                Ok(ParsedIndirect {
                    id,
                    node,
                    start,
                    end,
                    stream_data_start: None,
                })
            }
            Err(ParseError::Eof) if !self.lex.is_complete() => Err(ParseError::Eof),
            Err(_) => {
                let end = node.span.map_or(start, |s| s.end);
                self.diag(
                    DiagCode::MissingEndobj,
                    "missing 'endobj'",
                    Span::new(end, end),
                );
                Ok(ParsedIndirect {
                    id,
                    node,
                    start,
                    end,
                    stream_data_start: None,
                })
            }
        }
    }

    /// Position right after the EOL following the `stream` keyword.
    fn stream_data_start(&mut self, kw: Span) -> Result<u64, ParseError> {
        let buf = self.lex.buf();
        let mut p = self.lex.pos();
        // Tolerate spaces between `stream` and the EOL.
        let mut extra = false;
        while buf.get(p) == Some(&b' ') || buf.get(p) == Some(&b'\t') {
            p += 1;
            extra = true;
        }
        match (buf.get(p), buf.get(p + 1)) {
            (Some(b'\r'), Some(b'\n')) => p += 2,
            (Some(b'\n'), _) => p += 1,
            (Some(b'\r'), Some(_)) => {
                self.diag(
                    DiagCode::StreamEolInvalid,
                    "'stream' followed by CR only",
                    kw,
                );
                p += 1;
            }
            (None, _) | (Some(b'\r'), None) => return Err(ParseError::Eof),
            _ => {
                self.diag(
                    DiagCode::StreamEolInvalid,
                    "'stream' not followed by EOL",
                    kw,
                );
            }
        }
        if extra {
            self.diag(
                DiagCode::StreamEolInvalid,
                "whitespace before EOL after 'stream'",
                kw,
            );
        }
        self.lex.set_pos(p);
        Ok(self.lex.abs_pos())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::DictExt;

    fn parse(s: &str) -> Node {
        Parser::new(s.as_bytes(), 0, true).parse_object().unwrap()
    }

    #[test]
    fn refs_and_ints() {
        let n = parse("[1 2 0 R 3 4]");
        let a = n.obj.as_array().unwrap();
        assert_eq!(a.len(), 4);
        assert_eq!(a[1].obj, Object::Ref(ObjId::new(2, 0)));
        assert_eq!(a[2].obj, Object::Integer(3));
        assert_eq!(a[3].obj, Object::Integer(4));
    }

    #[test]
    fn dict_spans() {
        let src = "<< /A 1 /B [true null] >>";
        let n = parse(src);
        assert_eq!(n.span, Some(Span::new(0, src.len() as u64)));
        let d = n.obj.as_dict().unwrap();
        assert_eq!(d.get_i64("A"), Some(1));
        assert_eq!(
            d.get(b"B".as_slice()).unwrap().span,
            Some(Span::new(11, 22))
        );
    }

    #[test]
    fn truncated_needs_more() {
        let mut p = Parser::new(b"<< /A 1 0", 0, false);
        assert_eq!(p.parse_object(), Err(ParseError::Eof));
    }

    #[test]
    fn indirect_stream() {
        let src = b"7 0 obj\n<< /Length 3 >>\nstream\r\nabc\nendstream\nendobj";
        let mut p = Parser::new(src, 100, true);
        let r = p.parse_indirect().unwrap();
        assert_eq!(r.id, ObjId::new(7, 0));
        assert_eq!(r.stream_data_start, Some(100 + 32));
    }

    #[test]
    fn missing_endobj_tolerated() {
        let mut p = Parser::new(b"1 0 obj << /A 1 >> 2 0 obj", 0, false);
        let r = p.parse_indirect().unwrap();
        assert!(matches!(r.node.obj, Object::Dict(_)));
        assert!(
            p.take_diags()
                .iter()
                .any(|d| d.code == DiagCode::MissingEndobj)
        );
    }
}
