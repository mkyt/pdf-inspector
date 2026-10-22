//! The raw PDF object model.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjId {
    pub num: u32,
    pub generation: u16,
}

impl ObjId {
    pub const fn new(num: u32, generation: u16) -> Self {
        Self { num, generation }
    }
}

impl fmt::Display for ObjId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.num, self.generation)
    }
}

/// Byte range `[start, end)`. Unless stated otherwise it refers to the source
/// file; nodes of objects stored in object streams refer to the decoded
/// object stream data (see [`ObjLocation`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl Span {
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    pub const fn len(&self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(pub Vec<u8>);

impl Name {
    pub fn new(s: impl Into<Vec<u8>>) -> Self {
        Self(s.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_str_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.as_str_lossy())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StrKind {
    Literal,
    Hex,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PdfString {
    pub bytes: Vec<u8>,
    pub kind: StrKind,
}

impl PdfString {
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            kind: StrKind::Literal,
        }
    }
}

pub type Dict = IndexMap<Name, Node>;

/// Where a stream's (still encoded) data lives.
#[derive(Clone, PartialEq, Debug)]
pub enum StreamData {
    /// Range of the source file.
    Source(Span),
    /// Data held in memory (edited streams, inline data).
    Owned(Arc<[u8]>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Stream {
    pub dict: Dict,
    pub data: StreamData,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Object {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(PdfString),
    Name(Name),
    Array(Vec<Node>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjId),
}

impl Object {
    pub fn type_name(&self) -> &'static str {
        match self {
            Object::Null => "null",
            Object::Bool(_) => "boolean",
            Object::Integer(_) => "integer",
            Object::Real(_) => "real",
            Object::String(_) => "string",
            Object::Name(_) => "name",
            Object::Array(_) => "array",
            Object::Dict(_) => "dictionary",
            Object::Stream(_) => "stream",
            Object::Ref(_) => "reference",
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Object::Integer(i) => Some(i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Object::Integer(i) => Some(i as f64),
            Object::Real(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&Name> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Node]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_ref_id(&self) -> Option<ObjId> {
        match *self {
            Object::Ref(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Object::Stream(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&PdfString> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
}

/// An object together with its location.
#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    pub obj: Object,
    /// `None` for nodes created by edits.
    pub span: Option<Span>,
}

impl Node {
    pub fn new(obj: Object, span: Span) -> Self {
        Self {
            obj,
            span: Some(span),
        }
    }

    pub fn synthetic(obj: Object) -> Self {
        Self { obj, span: None }
    }
}

impl From<Object> for Node {
    fn from(obj: Object) -> Self {
        Self::synthetic(obj)
    }
}

/// Convenience lookups on dictionaries.
pub trait DictExt {
    fn get_key(&self, key: &str) -> Option<&Object>;
    fn get_i64(&self, key: &str) -> Option<i64>;
    fn get_name(&self, key: &str) -> Option<&Name>;
    fn type_is(&self, ty: &str) -> bool;
}

impl DictExt for Dict {
    fn get_key(&self, key: &str) -> Option<&Object> {
        self.get(key.as_bytes()).map(|n| &n.obj)
    }

    fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_key(key).and_then(Object::as_i64)
    }

    fn get_name(&self, key: &str) -> Option<&Name> {
        self.get_key(key).and_then(Object::as_name)
    }

    fn type_is(&self, ty: &str) -> bool {
        self.get_name("Type").is_some_and(|n| n == ty)
    }
}

impl std::borrow::Borrow<[u8]> for Name {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

/// Where an indirect object was read from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjLocation {
    /// `N G obj ... endobj` at this span of the file.
    File(Span),
    /// Stored in an object stream; spans of its nodes refer to the decoded
    /// data of `stream`.
    ObjStm { stream: ObjId, index: u32 },
}

#[derive(Clone, PartialEq, Debug)]
pub struct IndirectObject {
    pub id: ObjId,
    pub node: Node,
    pub location: ObjLocation,
}
