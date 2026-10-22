//! Tokenizer shared by the file parser and the content stream parser.

use crate::diag::{DiagCode, Diagnostic};
use crate::object::{PdfString, Span, StrKind};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok<'a> {
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(PdfString),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    BraceOpen,
    BraceClose,
    /// Any run of regular characters that is not a number: `obj`, `R`,
    /// `true`, content stream operators, ...
    Keyword(&'a [u8]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token<'a> {
    pub tok: Tok<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LexError {
    /// The buffer ended in the middle of a token (or before any token).
    Eof,
}

pub const fn is_whitespace(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

pub const fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

pub const fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Lexer<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Absolute offset of `buf[0]`.
    base: u64,
    /// The buffer holds all available data: a token touching its end is
    /// complete rather than truncated.
    complete: bool,
    pub diags: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    pub fn new(buf: &'a [u8], base: u64) -> Self {
        Self {
            buf,
            pos: 0,
            base,
            complete: false,
            diags: Vec::new(),
        }
    }

    pub fn new_complete(buf: &'a [u8], base: u64) -> Self {
        Self {
            complete: true,
            ..Self::new(buf, base)
        }
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn buf(&self) -> &'a [u8] {
        self.buf
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos.min(self.buf.len());
    }

    pub fn abs_pos(&self) -> u64 {
        self.base + self.pos as u64
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.base + start as u64, self.base + self.pos as u64)
    }

    fn diag(&mut self, code: DiagCode, msg: impl Into<String>, start: usize) {
        let span = self.span(start);
        self.diags
            .push(Diagnostic::warning(code, msg).with_span(span));
    }

    /// Skip whitespace and comments.
    pub fn skip_ws(&mut self) {
        while let Some(&b) = self.buf.get(self.pos) {
            if is_whitespace(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(&c) = self.buf.get(self.pos) {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn peek(&self) -> Result<Token<'a>, LexError> {
        self.clone_quiet().next_token()
    }

    fn clone_quiet(&self) -> Lexer<'a> {
        Lexer {
            diags: Vec::new(),
            ..*self
        }
    }

    pub fn next_token(&mut self) -> Result<Token<'a>, LexError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&b) = self.buf.get(self.pos) else {
            return Err(LexError::Eof);
        };
        let tok = match b {
            b'[' => {
                self.pos += 1;
                Tok::ArrayOpen
            }
            b']' => {
                self.pos += 1;
                Tok::ArrayClose
            }
            b'{' => {
                self.pos += 1;
                Tok::BraceOpen
            }
            b'}' => {
                self.pos += 1;
                Tok::BraceClose
            }
            b'<' => match self.buf.get(self.pos + 1) {
                Some(b'<') => {
                    self.pos += 2;
                    Tok::DictOpen
                }
                Some(_) => self.hex_string()?,
                None => return Err(LexError::Eof),
            },
            b'>' => match self.buf.get(self.pos + 1) {
                Some(b'>') => {
                    self.pos += 2;
                    Tok::DictClose
                }
                Some(_) => {
                    // Stray '>': report and treat it as a keyword so callers can recover.
                    self.pos += 1;
                    self.diag(DiagCode::UnexpectedToken, "unexpected '>'", start);
                    Tok::Keyword(&self.buf[start..self.pos])
                }
                None => return Err(LexError::Eof),
            },
            b'(' => self.literal_string()?,
            b')' => {
                self.pos += 1;
                self.diag(DiagCode::UnbalancedDelimiter, "unbalanced ')'", start);
                Tok::Keyword(&self.buf[start..self.pos])
            }
            b'/' => self.name()?,
            _ => self.regular()?,
        };
        Ok(Token {
            tok,
            span: self.span(start),
        })
    }

    fn regular(&mut self) -> Result<Tok<'a>, LexError> {
        let start = self.pos;
        while let Some(&b) = self.buf.get(self.pos) {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
        }
        if self.pos == self.buf.len() && !self.complete {
            // A token touching the end of the buffer may be truncated.
            return Err(LexError::Eof);
        }
        let word = &self.buf[start..self.pos];
        Ok(self.classify(word, start))
    }

    fn classify(&mut self, word: &'a [u8], start: usize) -> Tok<'a> {
        let first = word[0];
        if first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.') {
            if let Some(t) = self.number(word, start) {
                return t;
            }
        }
        Tok::Keyword(word)
    }

    fn number(&mut self, word: &[u8], start: usize) -> Option<Tok<'a>> {
        let mut i = 0;
        let mut neg = false;
        // Tolerate repeated signs ("--5", "+-5") as seen in the wild.
        let mut signs = 0;
        while i < word.len() && matches!(word[i], b'+' | b'-') {
            neg ^= word[i] == b'-';
            i += 1;
            signs += 1;
        }
        let digits = &word[i..];
        if digits.is_empty() {
            return None;
        }
        let mut int_part: i64 = 0;
        let mut overflow = false;
        let mut j = 0;
        while j < digits.len() && digits[j].is_ascii_digit() {
            match int_part
                .checked_mul(10)
                .and_then(|v| v.checked_add(i64::from(digits[j] - b'0')))
            {
                Some(v) => int_part = v,
                None => overflow = true,
            }
            j += 1;
        }
        let int_digits = j;
        let mut is_real = false;
        let mut frac_digits = 0;
        if j < digits.len() && digits[j] == b'.' {
            is_real = true;
            j += 1;
            while j < digits.len() && digits[j].is_ascii_digit() {
                j += 1;
                frac_digits += 1;
            }
        }
        if int_digits == 0 && frac_digits == 0 {
            return None;
        }
        if j != digits.len() {
            // Trailing garbage such as "1.2.3" or "12abc": accept the numeric
            // prefix only when the rest is still numeric-looking.
            if !digits[j..]
                .iter()
                .all(|b| b.is_ascii_digit() || *b == b'.' || *b == b'-')
            {
                return None;
            }
            self.diag(DiagCode::InvalidNumber, "malformed number", start);
        } else if signs > 1 {
            self.diag(DiagCode::InvalidNumber, "multiple signs in number", start);
        }
        if !is_real && !overflow {
            return Some(Tok::Int(if neg { -int_part } else { int_part }));
        }
        let text = std::str::from_utf8(&digits[..j]).ok()?;
        let text = text.strip_suffix('.').unwrap_or(text);
        let v: f64 = if text.is_empty() || text == "." {
            0.0
        } else {
            text.parse().ok()?
        };
        Some(Tok::Real(if neg { -v } else { v }))
    }

    fn name(&mut self) -> Result<Tok<'a>, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(&b) = self.buf.get(self.pos) {
            if !is_regular(b) {
                break;
            }
            if b == b'#' {
                let h1 = self.buf.get(self.pos + 1).copied();
                let h2 = self.buf.get(self.pos + 2).copied();
                match (h1.and_then(hex_val), h2.and_then(hex_val)) {
                    (Some(a), Some(c)) => {
                        out.push(a << 4 | c);
                        self.pos += 3;
                        continue;
                    }
                    _ if self.pos + 2 >= self.buf.len() && !self.complete => {
                        return Err(LexError::Eof);
                    }
                    _ => self.diag(DiagCode::InvalidName, "invalid #xx escape in name", start),
                }
            }
            out.push(b);
            self.pos += 1;
        }
        if self.pos == self.buf.len() && !self.complete {
            return Err(LexError::Eof);
        }
        Ok(Tok::Name(out))
    }

    fn hex_string(&mut self) -> Result<Tok<'a>, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        loop {
            let Some(&b) = self.buf.get(self.pos) else {
                return Err(LexError::Eof);
            };
            self.pos += 1;
            if b == b'>' {
                break;
            }
            if is_whitespace(b) {
                continue;
            }
            match hex_val(b) {
                Some(v) => match hi.take() {
                    Some(h) => out.push(h << 4 | v),
                    None => hi = Some(v),
                },
                None => self.diag(
                    DiagCode::InvalidString,
                    "invalid character in hex string",
                    start,
                ),
            }
        }
        if let Some(h) = hi {
            out.push(h << 4);
        }
        Ok(Tok::Str(PdfString {
            bytes: out,
            kind: StrKind::Hex,
        }))
    }

    fn literal_string(&mut self) -> Result<Tok<'a>, LexError> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1u32;
        loop {
            let Some(&b) = self.buf.get(self.pos) else {
                return Err(LexError::Eof);
            };
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(&e) = self.buf.get(self.pos) else {
                        return Err(LexError::Eof);
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'(' | b')' | b'\\' => out.push(e),
                        b'\r' => {
                            if self.buf.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.buf.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        // Unknown escape: the backslash is ignored.
                        _ => out.push(e),
                    }
                }
                b'\r' => {
                    // EOL inside a literal string is normalized to \n.
                    if self.buf.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        Ok(Tok::Str(PdfString {
            bytes: out,
            kind: StrKind::Literal,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok<'_>> {
        let mut l = Lexer::new_complete(s.as_bytes(), 0);
        let mut v = Vec::new();
        while let Ok(t) = l.next_token() {
            v.push(t.tok);
        }
        v
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(
            toks("<< /Type /Page /N 12 /R -3.5 /S (a\\(b\\)) /H <414> >> [1 0 R] % c\n true"),
            vec![
                Tok::DictOpen,
                Tok::Name(b"Type".to_vec()),
                Tok::Name(b"Page".to_vec()),
                Tok::Name(b"N".to_vec()),
                Tok::Int(12),
                Tok::Name(b"R".to_vec()),
                Tok::Real(-3.5),
                Tok::Name(b"S".to_vec()),
                Tok::Str(PdfString {
                    bytes: b"a(b)".to_vec(),
                    kind: StrKind::Literal
                }),
                Tok::Name(b"H".to_vec()),
                Tok::Str(PdfString {
                    bytes: vec![0x41, 0x40],
                    kind: StrKind::Hex
                }),
                Tok::DictClose,
                Tok::ArrayOpen,
                Tok::Int(1),
                Tok::Int(0),
                Tok::Keyword(b"R"),
                Tok::ArrayClose,
                Tok::Keyword(b"true"),
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            toks(".5 -.5 4. +7 --2"),
            vec![
                Tok::Real(0.5),
                Tok::Real(-0.5),
                Tok::Real(4.0),
                Tok::Int(7),
                Tok::Int(2)
            ]
        );
    }

    #[test]
    fn name_escapes() {
        assert_eq!(
            toks("/A#20B /#2F"),
            vec![Tok::Name(b"A B".to_vec()), Tok::Name(b"/".to_vec())]
        );
    }

    #[test]
    fn literal_escapes() {
        assert_eq!(
            toks("(\\101\\n\\\r\nx(y))"),
            vec![Tok::Str(PdfString::literal(b"A\nx(y)".to_vec()))]
        );
    }

    #[test]
    fn truncated_is_eof() {
        let mut l = Lexer::new(b"(abc", 0);
        assert_eq!(l.next_token(), Err(LexError::Eof));
        let mut l = Lexer::new(b"12", 0);
        assert_eq!(l.next_token(), Err(LexError::Eof));
    }
}
