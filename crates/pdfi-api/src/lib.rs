//! Frontend-facing API: a [`Session`] answering protocol [`Request`]s with
//! [`Response`]s and publishing [`Event`]s. Transports (WebSocket server,
//! wasm Worker) only move messages; all behavior lives here.

// Test support shared by the per-backend conformance runners.
#[doc(hidden)]
pub mod conformance;
mod convert;
pub mod dto;
pub mod job;
pub mod protocol;
mod session;
pub mod source;
mod ts;

pub use dto::*;
pub use job::{Executor, QueueExecutor, Task};
pub use protocol::*;
pub use session::{MAX_CHUNK, MAX_PAGE, Session, SubscriptionId};
pub use source::{NoFilesystem, Opened, SourceProvider};
pub use ts::typescript;
