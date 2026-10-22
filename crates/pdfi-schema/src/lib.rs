//! Known PDF object types: the Arlington PDF Model compiled into tables,
//! type inference along references, and validation.

pub mod check;
pub mod expr;
pub mod tables;

pub use check::*;
pub use pdfi_syntax::Severity;
pub use tables::{Base, KeyDef, Req, TypeDef, TypeId, Ver};
