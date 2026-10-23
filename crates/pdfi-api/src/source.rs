//! Access to files is supplied by the embedding layer (server, wasm, FFI);
//! this crate never touches the filesystem itself.

use std::sync::Arc;

use pdfi_syntax::ByteSource;

use crate::dto::{DirListing, OpenSource};
use crate::protocol::{ApiError, ErrorCode};

pub struct Opened {
    pub source: Arc<dyn ByteSource>,
    pub name: String,
    pub path: Option<String>,
}

pub trait SourceProvider: Send + Sync {
    /// Whether `OpenSource::Path` and `list_dir` are available.
    fn filesystem(&self) -> bool {
        false
    }

    fn open(&self, source: &OpenSource) -> Result<Opened, ApiError>;

    /// `None` lists the allowed roots.
    fn list_dir(&self, _path: Option<&str>) -> Result<DirListing, ApiError> {
        Err(ApiError::new(
            ErrorCode::Unsupported,
            "this backend cannot list directories",
        ))
    }

    /// Start writing a file at `path` (replacing it only on
    /// [`SaveTarget::commit`]).
    fn create(&self, _path: &str) -> Result<Box<dyn SaveTarget>, ApiError> {
        Err(ApiError::new(
            ErrorCode::Unsupported,
            "this backend cannot write files",
        ))
    }
}

/// A file being written. Dropping it without `commit` leaves nothing
/// behind (and the old file, if any, as it was).
pub trait SaveTarget: Send {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()>;

    /// Put the file in place and open it for reading.
    fn commit(self: Box<Self>) -> Result<Opened, ApiError>;
}

/// Provider that opens nothing; documents are added with
/// [`crate::Session::open_source`].
pub struct NoFilesystem;

impl SourceProvider for NoFilesystem {
    fn open(&self, _source: &OpenSource) -> Result<Opened, ApiError> {
        Err(ApiError::new(
            ErrorCode::Unsupported,
            "this backend cannot open paths",
        ))
    }
}
