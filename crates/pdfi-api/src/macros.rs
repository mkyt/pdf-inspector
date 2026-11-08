//! Macros run by the session as jobs (`JobKind::Macro`), and the stored
//! macros (`Macros`, `MacroSource`, `SaveMacro`, `DeleteMacro`).
//!
//! The JavaScript engine lives above this crate (pdfi-macro, native only):
//! an embedder installs a [`MacroHost`] with [`crate::Session::set_macro_host`].
//! Without one (the browser, which runs macros in a Worker of its own),
//! these requests are `Unsupported`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::dto::{DocId, JobId, LogLevel, MacroInfo, MacroResult};
use crate::job::Job;
use crate::protocol::{ApiError, Event};

/// Log lines a macro job sends as events; later ones are dropped
/// (`MacroResult.log_truncated`).
pub const MAX_MACRO_LOG: usize = 10_000;

/// Finds, stores and runs macros for a session.
pub trait MacroHost: Send + Sync {
    /// The stored macros, and where `save` writes (`None`: it cannot).
    fn list(&self) -> Result<(Vec<MacroInfo>, Option<String>), ApiError>;
    /// A stored macro by name, and its code.
    fn source(&self, name: &str) -> Result<(MacroInfo, String), ApiError>;
    /// Write `<name>.js` into the user's macro directory.
    fn save(&self, name: &str, code: &str) -> Result<MacroInfo, ApiError>;
    /// Remove `<name>.js` from the user's macro directory.
    fn delete(&self, name: &str) -> Result<(), ApiError>;
    /// Run a macro as one undoable step on `run.doc`, until it ends. Called
    /// on a thread of its own; fails with `ErrorCode::Cancelled` when it
    /// stopped because [`MacroRun::cancelled`] said so.
    fn run(&self, run: &MacroRun) -> Result<MacroOutput, ApiError>;
}

/// What a macro job produced: the result and the bytes of the files it
/// wrote (by `MacroResult.emitted` index).
pub struct MacroOutput {
    pub result: MacroResult,
    pub files: Vec<Vec<u8>>,
}

/// One macro job, as the [`MacroHost`] sees it.
pub struct MacroRun {
    pub job: JobId,
    pub doc: DocId,
    pub name: String,
    pub source: Option<String>,
    /// The raw parameters, a JSON object.
    pub params: String,
    /// The document's file name and size (`ctx.input`).
    pub input_name: String,
    pub input_size: u64,
    reporter: MacroReporter,
}

impl MacroRun {
    pub(crate) fn new(
        handle: Arc<Job>,
        name: String,
        source: Option<String>,
        params: String,
    ) -> Self {
        let info = handle.info();
        Self {
            job: info.job,
            doc: info.doc,
            name,
            source,
            params,
            input_name: handle.doc.info.name.clone(),
            input_size: handle.doc.info.size,
            reporter: MacroReporter {
                job: info.job,
                handle,
                logged: Arc::default(),
                truncated: Arc::default(),
            },
        }
    }

    /// The flag `CancelJob` (or closing the document) sets.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.reporter.handle.cancel_flag()
    }

    pub fn cancelled(&self) -> bool {
        self.cancel_flag().load(Ordering::Relaxed)
    }

    /// Where the job's log and progress go (can be moved into callbacks).
    pub fn reporter(&self) -> MacroReporter {
        self.reporter.clone()
    }
}

/// Sends a macro job's log lines (`MacroLog`) and progress (`JobProgress`).
#[derive(Clone)]
pub struct MacroReporter {
    job: JobId,
    handle: Arc<Job>,
    logged: Arc<AtomicUsize>,
    truncated: Arc<AtomicBool>,
}

impl MacroReporter {
    pub fn log(&self, level: LogLevel, text: &str) {
        if self.logged.fetch_add(1, Ordering::Relaxed) >= MAX_MACRO_LOG {
            self.truncated.store(true, Ordering::Relaxed);
            return;
        }
        self.handle.emit(&Event::MacroLog {
            job: self.job,
            level,
            text: text.to_string(),
        });
    }

    /// Whether log lines were dropped.
    pub fn log_truncated(&self) -> bool {
        self.truncated.load(Ordering::Relaxed)
    }

    pub fn progress(&self, done: u64, total: u64, label: Option<&str>) {
        self.handle.set_progress(done, total, label);
    }
}
