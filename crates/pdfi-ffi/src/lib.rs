//! In-process binding for native apps (macOS): a pdfi-api [`Session`]
//! exposed through UniFFI (docs/DESIGN.md §4.10).
//!
//! The protocol types are the binding's types (pdfi-api's `ffi` feature), so
//! requests go through [`PdfSession::dispatch`] as typed values, the same
//! [`Request`] / [`ResponseBody`] as the WebSocket and the Worker, without
//! JSON. Files are opened by path: the app hands over paths it holds access
//! to (security-scoped URLs), and saving writes to a location the app picked
//! (NSDocument replaces the original itself).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pdfi_api::{
    ApiError, DocId, DocSummary, ErrorCode, Event, OpenSource, Opened, PROTOCOL_VERSION, Request,
    Response, ResponseBody, SaveMode, SaveResult, Session, SourceProvider, SubscriptionId,
    ThreadExecutor,
};
use pdfi_syntax::FileSource;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PdfiError {
    #[error("{message}")]
    Api { code: ErrorCode, message: String },
}

impl From<ApiError> for PdfiError {
    fn from(e: ApiError) -> Self {
        Self::Api {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<std::io::Error> for PdfiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::from(e).into()
    }
}

/// The protocol version of these types (for `Request::Hello`).
#[uniffi::export]
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// A response body and the binary parts it refers to (by `part`).
#[derive(Debug, uniffi::Record)]
pub struct Reply {
    pub body: ResponseBody,
    pub blobs: Vec<Vec<u8>>,
}

/// Set from any thread to stop a running request (`Render`, `HitTest`) or a
/// save.
#[derive(Debug, Default, uniffi::Object)]
pub struct CancelToken(AtomicBool);

#[uniffi::export]
impl CancelToken {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Receives the session's events, on whatever thread emits them (a job's,
/// or the one dispatching the request).
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: Event);
}

/// Opens any path it is given: the app decides what the user may open.
/// Directory listing is left to the app's panels.
struct LocalFiles;

fn opened(path: &str) -> Result<Opened, ApiError> {
    let p = Path::new(path);
    let source = FileSource::open(p).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::NotFound
        } else {
            ErrorCode::Io
        };
        ApiError::new(code, format!("{path}: {e}"))
    })?;
    Ok(Opened {
        source: Arc::new(source),
        name: p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: Some(path.to_owned()),
    })
}

impl SourceProvider for LocalFiles {
    fn open(&self, source: &OpenSource) -> Result<Opened, ApiError> {
        let OpenSource::Path { path } = source;
        opened(path)
    }
}

#[derive(uniffi::Object)]
pub struct PdfSession {
    session: Session,
}

#[uniffi::export]
impl PdfSession {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            session: Session::new("native", LocalFiles, ThreadExecutor),
        })
    }

    /// Handle one request. Blocks until it is answered: call it off the
    /// main thread. Requests may run in parallel (tiles of a page).
    pub fn dispatch(&self, request: Request) -> Result<Reply, PdfiError> {
        reply(self.session.dispatch(request))
    }

    /// Like `dispatch`; `Render` and `HitTest` stop with `cancelled` once
    /// `cancel` is set.
    pub fn dispatch_cancellable(
        &self,
        request: Request,
        cancel: Arc<CancelToken>,
    ) -> Result<Reply, PdfiError> {
        reply(
            self.session
                .dispatch_cancellable(request, &|| cancel.is_cancelled()),
        )
    }

    pub fn subscribe(&self, listener: Arc<dyn EventListener>) -> SubscriptionId {
        self.session
            .subscribe(move |ev| listener.on_event(ev.clone()))
    }

    pub fn unsubscribe(&self, id: SubscriptionId) {
        self.session.unsubscribe(id);
    }

    /// Write a document with its edits to a new file at `path` (it must not
    /// exist). Nothing is left there on failure or when cancelled. The
    /// document stays as it was; `reload` it from where the file ends up.
    pub fn save_to(
        &self,
        doc: DocId,
        mode: SaveMode,
        path: String,
        cancel: Arc<CancelToken>,
    ) -> Result<SaveResult, PdfiError> {
        let mut stream = self.session.save_stream(doc, mode)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let written = (|| -> Result<(), PdfiError> {
            while let Some(chunk) = stream.next_chunk()? {
                if cancel.is_cancelled() {
                    return Err(ApiError::new(ErrorCode::Cancelled, "cancelled").into());
                }
                file.write_all(&chunk)?;
            }
            file.sync_all()?;
            Ok(())
        })();
        drop(file);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(SaveResult {
            path: Some(path),
            ..stream.result()
        })
    }

    /// Replace a document by the file at `path` (after saving it there),
    /// with a fresh history.
    pub fn reload(&self, doc: DocId, path: String) -> Result<DocSummary, PdfiError> {
        Ok(self.session.reload_from(doc, opened(&path)?)?)
    }
}

fn reply(r: pdfi_api::Reply) -> Result<Reply, PdfiError> {
    match r.response {
        Response::Ok(body) => Ok(Reply {
            body,
            blobs: r.blobs,
        }),
        Response::Err(e) => Err(e.into()),
    }
}
