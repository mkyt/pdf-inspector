//! Conversions from the syntax layer to DTOs.

use pdfi_syntax::{self as syn, DictExt, Object};

use crate::dto::*;

/// Bytes of a string shown in a [`Value::String`].
const STRING_PREVIEW: usize = 1024;

/// `#xx`-escaped name without the leading `/`.
pub fn name_text(n: &syn::Name) -> String {
    let mut out = Vec::new();
    syn::write::write_name(n, &mut out);
    String::from_utf8_lossy(&out[1..]).into_owned()
}

/// Decode a text string: UTF-16BE or UTF-8 with BOM, PDFDocEncoding otherwise
/// (approximated by Latin-1 plus the 0x80..0x9f punctuation block).
pub fn text_of(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdfdoc_char(b)).collect()
}

fn pdfdoc_char(b: u8) -> char {
    const HIGH: [char; 32] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚',
        '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{fffd}',
    ];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        0xa0 => '€',
        _ => b as char,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn type_name(d: &syn::Dict) -> Option<String> {
    d.get_name("Type").map(name_text)
}

/// Length of stored stream data, without reading it.
pub fn raw_len(s: &syn::Stream) -> u64 {
    match &s.data {
        syn::StreamData::Source(span) => span.len(),
        syn::StreamData::Owned(d) => d.len() as u64,
    }
}

pub fn value_of(o: &Object) -> Value {
    match o {
        Object::Null => Value::Null,
        Object::Bool(b) => Value::Bool { value: *b },
        Object::Integer(i) => Value::Integer { value: *i },
        Object::Real(r) => Value::Real { value: *r },
        Object::String(s) => {
            let shown = &s.bytes[..s.bytes.len().min(STRING_PREVIEW)];
            Value::String {
                kind: match s.kind {
                    syn::StrKind::Literal => StringKind::Literal,
                    syn::StrKind::Hex => StringKind::Hex,
                },
                len: s.bytes.len() as u64,
                text: text_of(shown),
                hex: hex(shown),
                truncated: shown.len() < s.bytes.len(),
            }
        }
        Object::Name(n) => Value::Name { name: name_text(n) },
        Object::Array(a) => Value::Array {
            len: a.len() as u64,
        },
        Object::Dict(d) => Value::Dict {
            len: d.len() as u64,
            type_name: type_name(d),
        },
        Object::Stream(s) => Value::Stream {
            len: s.dict.len() as u64,
            type_name: type_name(&s.dict),
            raw_len: raw_len(s),
        },
        Object::Ref(id) => Value::Ref { id: (*id).into() },
    }
}

pub fn xref_entry(e: syn::xref::XrefEntry) -> XrefEntry {
    use syn::xref::XrefEntry as E;
    match e {
        E::Free { next, generation } => XrefEntry::Free { next, generation },
        E::InUse { offset, generation } => XrefEntry::InUse { offset, generation },
        E::Compressed { stream, index } => XrefEntry::Compressed { stream, index },
    }
}

pub fn xref_kind(k: syn::xref::XrefKind) -> XrefKind {
    use syn::xref::XrefKind as K;
    match k {
        K::Table => XrefKind::Table,
        K::Stream => XrefKind::Stream,
        K::Hybrid => XrefKind::Hybrid,
        K::Recovered => XrefKind::Recovered,
    }
}

pub fn diagnostic(d: &syn::Diagnostic) -> DiagnosticInfo {
    DiagnosticInfo {
        severity: match d.severity {
            syn::Severity::Info => Severity::Info,
            syn::Severity::Warning => Severity::Warning,
            syn::Severity::Error => Severity::Error,
        },
        code: format!("{:?}", d.code),
        message: d.message.clone(),
        obj: d.obj.map(Into::into),
        span: d.span.map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_strings() {
        assert_eq!(text_of(b"\xfe\xff\x00A\x00B"), "AB");
        assert_eq!(text_of(b"\xef\xbb\xbf\xe3\x81\x82"), "あ");
        assert_eq!(text_of(b"a\x84b"), "a—b");
    }

    #[test]
    fn names_are_escaped() {
        assert_eq!(name_text(&syn::Name::new(b"A B#".to_vec())), "A#20B#23");
    }
}
