//! Diagnostics collected while reading a file.
//!
//! The reader is tolerant: problems are recorded here and parsing continues
//! wherever a reasonable interpretation exists.

use std::sync::Mutex;

use crate::object::{ObjId, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Stable machine-readable identifier of a diagnostic kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagCode {
    HeaderMissing,
    HeaderOffset,
    StartXrefMissing,
    XrefInvalid,
    XrefOffsetAdjusted,
    XrefLoop,
    XrefEntryMismatch,
    TrailerInvalid,
    RecoveryUsed,
    SyntaxError,
    UnexpectedToken,
    UnbalancedDelimiter,
    MissingEndobj,
    MissingEndstream,
    StreamLengthInvalid,
    StreamEolInvalid,
    InvalidNumber,
    InvalidName,
    InvalidString,
    NestingTooDeep,
    FilterError,
    FilterUnsupported,
    ObjStmInvalid,
    EncryptionError,
    ObjectMissing,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagCode,
    pub message: String,
    pub obj: Option<ObjId>,
    /// Byte range in the source file, when the problem can be located there.
    pub span: Option<Span>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            obj: None,
            span: None,
        }
    }

    pub fn warning(code: DiagCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn error(code: DiagCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn with_obj(mut self, obj: ObjId) -> Self {
        self.obj = Some(obj);
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Thread-safe append-only diagnostic sink.
#[derive(Debug, Default)]
pub struct DiagSink {
    items: Mutex<Vec<Diagnostic>>,
}

impl DiagSink {
    pub fn push(&self, d: Diagnostic) {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).push(d);
    }

    pub fn extend(&self, ds: impl IntoIterator<Item = Diagnostic>) {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(ds);
    }

    pub fn len(&self) -> usize {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> Vec<Diagnostic> {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
