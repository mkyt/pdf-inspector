//! Conversions from the syntax layer to DTOs.

use pdfi_schema as schema;
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

pub use pdfi_model::text::decode as text_of;

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
        E::Edited { generation } => XrefEntry::Edited { generation },
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

fn severity(s: syn::Severity) -> Severity {
    match s {
        syn::Severity::Info => Severity::Info,
        syn::Severity::Warning => Severity::Warning,
        syn::Severity::Error => Severity::Error,
    }
}

pub fn diagnostic(d: &syn::Diagnostic) -> DiagnosticInfo {
    DiagnosticInfo {
        severity: severity(d.severity),
        code: format!("{:?}", d.code),
        message: d.message.clone(),
        obj: d.obj.map(Into::into),
        path: None,
        span: d.span.map(Into::into),
    }
}

/// A schema issue. `in_file`: spans refer to the source file (not to an
/// object stream's data).
pub fn issue(i: &schema::Issue, in_file: bool) -> DiagnosticInfo {
    DiagnosticInfo {
        severity: severity(i.severity),
        code: format!("{:?}", i.code),
        message: i.message.clone(),
        obj: i.obj.map(Into::into),
        path: (!i.path.is_empty()).then(|| schema::path_text(&i.path)),
        span: i.span.filter(|_| in_file).map(Into::into),
    }
}

/// Allowed values listed in a key definition.
const MAX_ALLOWED: usize = 64;

pub fn key_doc(k: &schema::KeyDef) -> KeyDoc {
    use schema::tables::Lit;
    let ver = |v: u8| (v != 0).then(|| schema::version_text(v));
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let mut allowed = Vec::new();
    for slot in k.slots {
        for l in slot.values.iter().flat_map(|v| v.lits) {
            if allowed.len() == MAX_ALLOWED {
                break;
            }
            allowed.push(match l.lit {
                Lit::Name(n) => format!("/{n}"),
                Lit::Int(i) => i.to_string(),
                Lit::Num(n) => n.to_string(),
                Lit::Str(s) => format!("({s})"),
                Lit::Bool(b) => b.to_string(),
            });
        }
    }
    KeyDoc {
        types: k
            .slots
            .iter()
            .map(|s| SlotDoc {
                base: s.base.as_str().into(),
                links: s
                    .links
                    .iter()
                    .map(|l| schema::type_def(l.ty).name.to_string())
                    .collect(),
            })
            .collect(),
        requirement: match k.required {
            schema::Req::Yes => Requirement::Required,
            schema::Req::No => Requirement::Optional,
            schema::Req::Cond(c) => Requirement::Conditional {
                condition: c.into(),
            },
        },
        since: ver(k.since),
        deprecated: ver(k.deprecated),
        extension: text(k.extension),
        inheritable: k.inheritable,
        default: text(k.default),
        allowed,
        note: text(k.note),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_escaped() {
        assert_eq!(name_text(&syn::Name::new(b"A B#".to_vec())), "A#20B#23");
    }
}
