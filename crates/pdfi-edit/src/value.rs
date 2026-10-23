//! Values typed as PDF syntax, and text strings.

use pdfi_syntax::parser::{ParseError, Parser};
use pdfi_syntax::{Object, PdfString, StrKind};

use crate::{EditError, Result};

/// Parse one object written in PDF syntax (`<< /A [1 2 0 R] >>`). Anything
/// after it, or a syntax warning (a malformed name, an unbalanced string),
/// is an error: what is stored should be what was typed.
pub fn parse_value(text: &str) -> Result<Object> {
    let bytes = text.trim().as_bytes();
    if bytes.is_empty() {
        return Err(EditError("empty value".into()));
    }
    let mut p = Parser::new(bytes, 0, true);
    let node = p.parse_object().map_err(|e| match e {
        ParseError::Eof => EditError("the value is incomplete".into()),
        e => EditError(e.to_string()),
    })?;
    if let Some(d) = p.take_diags().first() {
        return Err(EditError(d.message.clone()));
    }
    match p.next_tok() {
        Err(ParseError::Eof) => Ok(node.obj),
        Ok(t) => Err(EditError(format!(
            "unexpected text after the value at offset {}",
            t.span.start
        ))),
        Err(e) => Err(EditError(e.to_string())),
    }
}

/// A text string: PDFDocEncoding when the text is printable ASCII,
/// UTF-16BE with a byte order mark otherwise.
pub fn text_string(s: &str) -> PdfString {
    let bytes = if s.bytes().all(|b| (0x20..0x7f).contains(&b) || b == b'\n') {
        s.as_bytes().to_vec()
    } else {
        let mut v = vec![0xfe, 0xff];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    };
    PdfString {
        bytes,
        kind: StrKind::Literal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(parse_value(" 12 ").unwrap(), Object::Integer(12));
        assert_eq!(
            parse_value("3 0 R").unwrap(),
            Object::Ref(pdfi_syntax::ObjId::new(3, 0))
        );
        assert!(matches!(
            parse_value("<< /A [1 2.5 (x)] >>").unwrap(),
            Object::Dict(_)
        ));
        assert!(parse_value("[1 2").is_err());
        assert!(parse_value("1 2 3").is_err());
        assert!(parse_value("").is_err());
        assert!(parse_value("endobj").is_err());
    }

    #[test]
    fn text() {
        assert_eq!(text_string("abc").bytes, b"abc");
        assert_eq!(text_string("é").bytes, vec![0xfe, 0xff, 0, 0xe9]);
    }
}
