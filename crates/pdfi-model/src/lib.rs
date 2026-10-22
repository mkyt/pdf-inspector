//! The semantic layer over the raw objects: pages (inherited attributes
//! resolved, looked up by index through `/Count`), document metadata (Info
//! and XMP), outline, annotations and destinations, and the inventory of
//! fonts, images and references, which needs every object and is built by
//! a job ([`inventory::Builder`]).

pub mod annot;
pub mod dest;
pub mod inventory;
pub mod meta;
pub mod outline;
pub mod pages;
pub mod text;
pub mod tree;
mod util;
pub mod xmp;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

pub use pdfi_syntax::Severity;
use pdfi_syntax::{ObjId, PdfFile};

pub use util::{Rect, Res, resolve};

/// A problem found while building the model (the page tree's counts, the
/// metadata, ...). Schema problems are `pdfi-schema`'s business.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub obj: Option<ObjId>,
}

impl Issue {
    pub fn warning(code: &'static str, message: impl Into<String>, obj: Option<ObjId>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            obj,
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>, obj: Option<ObjId>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            obj,
        }
    }
}

/// Issues reported once per document.
const MAX_ISSUES: usize = 1000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Per-document state: indexes built on demand and kept for the lifetime of
/// the document. Pair it with the file through [`DocModel::on`].
#[derive(Default)]
pub struct DocModel {
    nodes: Mutex<HashMap<ObjId, Arc<pages::NodeIndex>>>,
    labels: OnceLock<Vec<pages::LabelRange>>,
    /// Outline items under a parent (`None`: the outline root).
    outline: Mutex<HashMap<Option<ObjId>, Arc<Vec<ObjId>>>>,
    metadata: OnceLock<Option<Arc<meta::Metadata>>>,
    issues: Mutex<(Vec<Issue>, HashSet<String>)>,
}

impl DocModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on<'a>(&'a self, file: &'a PdfFile) -> Model<'a> {
        Model { file, m: self }
    }
}

/// The model of one document.
#[derive(Clone, Copy)]
pub struct Model<'a> {
    pub file: &'a PdfFile,
    m: &'a DocModel,
}

impl<'a> Model<'a> {
    /// Issues found so far (they accumulate as parts of the model are built).
    pub fn issues(&self) -> Vec<Issue> {
        lock(&self.m.issues).0.clone()
    }

    pub(crate) fn report(&self, i: Issue) {
        let mut g = lock(&self.m.issues);
        if g.0.len() < MAX_ISSUES && g.1.insert(i.message.clone()) {
            g.0.push(i);
        }
    }

    /// The catalog (`/Root`), if it is a dictionary.
    pub fn catalog(&self) -> Option<(Option<ObjId>, Res<'a>)> {
        let root = pdfi_syntax::DictExt::get_key(self.file.trailer(), "Root")?;
        let (id, r) = resolve(self.file, root);
        r.as_dict().is_some().then_some((id, r))
    }
}
