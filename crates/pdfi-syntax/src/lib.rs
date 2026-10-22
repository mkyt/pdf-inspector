//! Low-level PDF syntax: byte sources, lexer, parser, cross-reference
//! tables, filters, encryption and serialization.

pub mod crypt;
pub mod diag;
pub mod file;
pub mod filters;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod scan;
pub mod source;
pub mod write;
pub mod xref;

pub use diag::{DiagCode, Diagnostic, Severity};
pub use file::{DecodedStream, Error, OpenOptions, PdfFile, Recovery};
pub use object::*;
#[cfg(any(unix, windows))]
pub use source::FileSource;
pub use source::{ByteSource, MemSource};
