//! Macros: JavaScript run next to the [`pdfi_api::Session`] holding the
//! document, on QuickJS (docs/DESIGN.md §4.11).
//!
//! A macro is one ES module: `export const meta` says what it is and which
//! parameters it takes, the default export does the work through the
//! prelude's API (`js/prelude.js`, typed and documented in `js/pdfi.d.ts`).
//! The prelude talks to the session through a few host functions
//! ([`engine`]); one run is one undoable step, reverted when it fails.
//!
//! Native only: the browser runs the same prelude in a Worker of its own.

pub mod engine;
mod library;
pub mod meta;
pub mod params;

pub use engine::{
    Emitted, Level, LogFn, Outcome, ProgressFn, RunOptions, check_params, read_meta, run,
};
pub use library::{Library, Listed, MacroSource, Origin};
pub use meta::{FullOptions, Meta, Param, ParamType, SaveKind, SaveSpec};

/// The public API, evaluated before every macro.
pub const PRELUDE: &str = include_str!("../js/prelude.js");

/// Types and documentation of the API, for editors (`pdfi macro types`).
pub const TYPES: &str = include_str!("../js/pdfi.d.ts");

/// The API version the prelude provides (`meta.api`).
pub const API: u32 = 1;

/// Macros built into the binary: `(file name, source)`.
pub const BUNDLED: &[(&str, &str)] = &[
    ("fonts.js", include_str!("../macros/fonts.js")),
    ("metadata.js", include_str!("../macros/metadata.js")),
    ("set-reading.js", include_str!("../macros/set-reading.js")),
    (
        "crop-to-content.js",
        include_str!("../macros/crop-to-content.js"),
    ),
    (
        "remove-owner-password.js",
        include_str!("../macros/remove-owner-password.js"),
    ),
];

/// What `pdfi macro new` writes; `NAME` is replaced.
pub const TEMPLATE: &str = include_str!("../js/template.js");

#[derive(Debug)]
pub enum MacroError {
    /// The macro threw (or failed to compile).
    Script {
        code: String,
        message: String,
        stack: Option<String>,
    },
    /// Its `meta` or the parameters given are not right.
    Invalid(String),
    Cancelled,
    Timeout,
    /// The session refused (another macro is editing the document, ...).
    Api(pdfi_api::ApiError),
    NotFound(String),
    Io(std::io::Error),
    /// Something wrong on the host's side.
    Host(String),
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MacroError::Script { message, stack, .. } => {
                f.write_str(message)?;
                if let Some(s) = stack {
                    let s = s.trim_end();
                    if !message.contains(s) {
                        write!(f, "\n{s}")?;
                    }
                }
                Ok(())
            }
            MacroError::Invalid(m) | MacroError::NotFound(m) | MacroError::Host(m) => {
                f.write_str(m)
            }
            MacroError::Cancelled => f.write_str("cancelled"),
            MacroError::Timeout => f.write_str("timed out"),
            MacroError::Api(e) => write!(f, "{}", e.message),
            MacroError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MacroError {}

impl From<std::io::Error> for MacroError {
    fn from(e: std::io::Error) -> Self {
        MacroError::Io(e)
    }
}
