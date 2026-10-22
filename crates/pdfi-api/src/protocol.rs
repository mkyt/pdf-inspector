//! The message protocol shared by every transport (WebSocket, Worker).
//!
//! Control messages are JSON: the client sends [`ClientMessage`], the backend
//! answers with [`ServerMessage`]. Binary payloads are sent as separate
//! [`BinaryFrame`]s *before* the response that refers to them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::dto::*;

/// Bumped on every incompatible change; checked by `Hello`.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClientMessage {
    pub id: RequestId,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    ListDir {
        path: Option<String>,
    },
    Open {
        source: OpenSource,
        password: Option<String>,
        /// Ignore the cross-reference data and rebuild it by scanning.
        recover: bool,
    },
    Docs,
    Close {
        doc: DocId,
    },
    Summary {
        doc: DocId,
    },
    /// Cross-reference rows in object-number order (merged table).
    Objects {
        doc: DocId,
        range: Range,
    },
    Object {
        doc: DocId,
        id: ObjRef,
    },
    Children {
        doc: DocId,
        path: NodePath,
        range: Range,
    },
    /// Entries of one revision (0 = newest), or of the merged table.
    Xref {
        doc: DocId,
        revision: Option<u32>,
        range: Range,
    },
    Diagnostics {
        doc: DocId,
        range: Range,
    },
    RawBytes {
        doc: DocId,
        offset: u64,
        len: u32,
    },
    StreamData {
        doc: DocId,
        id: ObjRef,
        /// Decrypt and apply filters; otherwise the bytes as stored.
        decoded: bool,
        offset: u64,
        len: u32,
    },
    StartJob {
        doc: DocId,
        job: JobKind,
    },
    JobStatus {
        job: JobId,
    },
    CancelJob {
        job: JobId,
    },
    /// Drop a request that has not started yet. Handled by the transport.
    Cancel {
        request: RequestId,
    },
    Batch {
        requests: Vec<Request>,
    },
}

// Responses are short-lived and moved once; boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Response {
    Ok(ResponseBody),
    Err(ApiError),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ResponseBody {
    Hello(HelloInfo),
    Dir(DirListing),
    Doc(DocSummary),
    Docs { docs: Vec<DocInfo> },
    Objects(Page<ObjectSummary>),
    Object(ObjectView),
    Children(Page<NodeView>),
    Xref(Page<XrefRow>),
    Diagnostics(Page<DiagnosticInfo>),
    Bytes(BytesInfo),
    Job(JobInfo),
    Batch { responses: Vec<Response> },
    Empty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HelloInfo {
    pub protocol: u32,
    pub backend: String,
    pub version: String,
    /// Backend can open paths (`OpenSource::Path`) and list directories.
    pub filesystem: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    BadRequest,
    VersionMismatch,
    NoSuchDoc,
    NoSuchJob,
    NotFound,
    Forbidden,
    Unsupported,
    TooLarge,
    Io,
    Parse,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<pdfi_syntax::Error> for ApiError {
    fn from(e: pdfi_syntax::Error) -> Self {
        let code = match &e {
            pdfi_syntax::Error::Io(_) => ErrorCode::Io,
            pdfi_syntax::Error::Missing(_) => ErrorCode::NotFound,
            pdfi_syntax::Error::Object(..)
            | pdfi_syntax::Error::Parse(_)
            | pdfi_syntax::Error::Invalid(_) => ErrorCode::Parse,
        };
        Self::new(code, e.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerMessage {
    Response { id: RequestId, response: Response },
    Event { event: Event },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    DocOpened {
        info: DocInfo,
    },
    DocClosed {
        doc: DocId,
    },
    JobProgress {
        job: JobId,
        doc: DocId,
        done: u64,
        total: u64,
    },
    JobDone {
        info: JobInfo,
    },
}

/// Binary payload of a response: `id` (u64 LE) + `part` (u32 LE) + bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryFrame {
    pub id: RequestId,
    pub part: u32,
    pub data: Vec<u8>,
}

impl BinaryFrame {
    pub const HEADER_LEN: usize = 12;

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::HEADER_LEN + self.data.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.part.to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::HEADER_LEN {
            return None;
        }
        let id = u64::from_le_bytes(buf[..8].try_into().ok()?);
        let part = u32::from_le_bytes(buf[8..12].try_into().ok()?);
        Some(Self {
            id,
            part,
            data: buf[12..].to_vec(),
        })
    }
}

/// What `dispatch` produces: the response plus the binary parts it refers to
/// (indexed by [`BytesInfo::part`]).
#[derive(Debug)]
pub struct Reply {
    pub response: Response,
    pub blobs: Vec<Vec<u8>>,
}

impl Reply {
    /// Frames to send for request `id`, binary parts first.
    pub fn into_frames(self, id: RequestId) -> (Vec<BinaryFrame>, ServerMessage) {
        let frames = self
            .blobs
            .into_iter()
            .enumerate()
            .map(|(i, data)| BinaryFrame {
                id,
                part: i as u32,
                data,
            })
            .collect();
        (
            frames,
            ServerMessage::Response {
                id,
                response: self.response,
            },
        )
    }
}
