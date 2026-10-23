//! Browser binding: a pdfi-api [`Session`] driven from a Web Worker.
//!
//! The Worker owns one [`WasmSession`]. It passes protocol messages as JSON
//! (the same [`ClientMessage`] / [`ServerMessage`] as the WebSocket), opens
//! files through a JS byte source (`Blob.slice` + `FileReaderSync`, so the
//! file is never copied into wasm memory) and runs jobs cooperatively between
//! messages with [`WasmSession::run_job_step`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use js_sys::Uint8Array;
use pdfi_api::{
    NoFilesystem, QueueExecutor, Reply, RequestId, Response, ResponseBody, SaveMode, SaveStream,
    ServerMessage, Session,
};
use pdfi_syntax::ByteSource;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TS_SOURCE: &str = r#"
/** Random-access bytes: `read` returns at most `length` bytes at `offset`. */
export interface JsByteSource {
    readonly size: number;
    read(offset: number, length: number): Uint8Array;
}
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "JsByteSource")]
    pub type JsByteSource;

    #[wasm_bindgen(method, getter)]
    fn size(this: &JsByteSource) -> f64;

    #[wasm_bindgen(method, catch)]
    fn read(this: &JsByteSource, offset: f64, length: u32) -> Result<Uint8Array, JsValue>;
}

// JS objects cannot cross threads, but `ByteSource` must be `Send + Sync`.
// Sources therefore live in a per-thread table and `JsSource` holds only
// the key; wasm32 without atomics has a single thread.
thread_local! {
    static SOURCES: RefCell<HashMap<u32, JsByteSource>> = RefCell::default();
}
static NEXT_SOURCE: AtomicU32 = AtomicU32::new(1);

/// Largest single read handed to JS (bounds the temporary `ArrayBuffer`).
const MAX_READ: usize = 4 << 20;

struct JsSource {
    key: u32,
    len: u64,
}

impl JsSource {
    fn new(src: JsByteSource) -> Self {
        let key = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
        let len = src.size().max(0.0) as u64;
        SOURCES.with_borrow_mut(|m| m.insert(key, src));
        Self { key, len }
    }
}

impl Drop for JsSource {
    fn drop(&mut self) {
        SOURCES.with_borrow_mut(|m| m.remove(&self.key));
    }
}

fn js_error(e: JsValue) -> io::Error {
    let msg = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| "read failed".into());
    io::Error::other(msg)
}

impl ByteSource for JsSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let want = buf
            .len()
            .min(MAX_READ)
            .min(usize::try_from(self.len - offset).unwrap_or(usize::MAX));
        let data = SOURCES.with_borrow(|m| {
            let src = m
                .get(&self.key)
                .ok_or_else(|| io::Error::other("source released"))?;
            src.read(offset as f64, want as u32).map_err(js_error)
        })?;
        let n = (data.length() as usize).min(want);
        data.subarray(0, n as u32).copy_to(&mut buf[..n]);
        Ok(n)
    }
}

/// A response ready to post: the JSON [`ServerMessage`] and its binary parts
/// (indexed by `BytesInfo.part`).
#[wasm_bindgen]
pub struct WasmReply {
    message: String,
    blobs: Vec<Vec<u8>>,
}

#[wasm_bindgen]
impl WasmReply {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    /// Fresh `Uint8Array`s (each with its own buffer, so they can be
    /// transferred).
    #[wasm_bindgen(getter)]
    pub fn blobs(&self) -> Vec<Uint8Array> {
        self.blobs
            .iter()
            .map(|b| Uint8Array::from(&b[..]))
            .collect()
    }
}

impl WasmReply {
    fn new(id: RequestId, reply: Reply) -> Self {
        let (frames, msg) = reply.into_frames(id);
        Self {
            message: json(&msg),
            blobs: frames.into_iter().map(|f| f.data).collect(),
        }
    }
}

fn json(msg: &ServerMessage) -> String {
    serde_json::to_string(msg).expect("protocol messages serialize")
}

#[wasm_bindgen]
pub struct WasmSession {
    session: Session,
    exec: Arc<QueueExecutor>,
    events: Arc<Mutex<Vec<String>>>,
    saves: RefCell<HashMap<u32, SaveStream>>,
    next_save: u32,
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

impl Default for WasmSession {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl WasmSession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        let exec = Arc::new(QueueExecutor::default());
        let session = Session::new("wasm", NoFilesystem, exec.clone());
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        {
            let events = events.clone();
            session.subscribe(move |ev| {
                let msg = json(&ServerMessage::Event { event: ev.clone() });
                events.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
            });
        }
        Self {
            session,
            exec,
            events,
            saves: RefCell::default(),
            next_save: 1,
        }
    }

    /// Handle one JSON-encoded `ClientMessage`.
    pub fn dispatch(&self, message: &str) -> WasmReply {
        let (id, reply) = self.session.dispatch_json(message);
        WasmReply::new(id, reply)
    }

    /// Open a document from a JS byte source; answers request `id` like
    /// `Request::Open` would.
    pub fn open(
        &self,
        id: f64,
        source: JsByteSource,
        name: String,
        password: Option<String>,
        recover: bool,
    ) -> WasmReply {
        let src: Arc<dyn ByteSource> = Arc::new(JsSource::new(source));
        let response = match self.session.open_source(src, name, password, recover) {
            Ok(s) => Response::Ok(ResponseBody::Doc(s)),
            Err(e) => Response::Err(e),
        };
        WasmReply::new(
            id as RequestId,
            Reply {
                response,
                blobs: Vec::new(),
            },
        )
    }

    /// Start writing a document with its edits (`mode`: a JSON `SaveMode`).
    /// Returns a handle for `saveChunk`.
    #[wasm_bindgen(js_name = saveBegin)]
    pub fn save_begin(&mut self, doc: u32, mode: &str) -> Result<u32, JsError> {
        let mode: SaveMode = serde_json::from_str(mode).map_err(js_err)?;
        let stream = self.session.save_stream(doc, mode).map_err(js_err)?;
        let id = self.next_save;
        self.next_save += 1;
        self.saves.borrow_mut().insert(id, stream);
        Ok(id)
    }

    /// The next part of the file (about 1 MB), or `undefined` at the end.
    #[wasm_bindgen(js_name = saveChunk)]
    pub fn save_chunk(&self, save: u32) -> Result<Option<Uint8Array>, JsError> {
        let mut saves = self.saves.borrow_mut();
        let s = saves
            .get_mut(&save)
            .ok_or_else(|| JsError::new("no such save"))?;
        Ok(s.next_chunk()
            .map_err(js_err)?
            .map(|c| Uint8Array::from(&c[..])))
    }

    /// Forget a save; returns its result (a JSON `SaveResult`).
    #[wasm_bindgen(js_name = saveEnd)]
    pub fn save_end(&self, save: u32) -> Result<String, JsError> {
        let s = self
            .saves
            .borrow_mut()
            .remove(&save)
            .ok_or_else(|| JsError::new("no such save"))?;
        serde_json::to_string(&s.result()).map_err(js_err)
    }

    /// Replace a document by the file it was saved to; answers request
    /// `id` like `Request::Open`.
    pub fn reload(&self, id: f64, doc: u32, source: JsByteSource, name: String) -> WasmReply {
        let src: Arc<dyn ByteSource> = Arc::new(JsSource::new(source));
        let response = match self.session.reload(doc, src, name) {
            Ok(s) => Response::Ok(ResponseBody::Doc(s)),
            Err(e) => Response::Err(e),
        };
        WasmReply::new(
            id as RequestId,
            Reply {
                response,
                blobs: Vec::new(),
            },
        )
    }

    /// Run one bounded step of the next job. Returns whether jobs remain.
    #[wasm_bindgen(js_name = runJobStep)]
    pub fn run_job_step(&self) -> bool {
        self.exec.run_step()
    }

    /// Events emitted since the last call, as JSON `ServerMessage`s.
    #[wasm_bindgen(js_name = takeEvents)]
    pub fn take_events(&self) -> Vec<String> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
