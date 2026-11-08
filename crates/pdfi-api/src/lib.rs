//! Frontend-facing API: a [`Session`] answering protocol [`Request`]s with
//! [`Response`]s and publishing [`Event`]s. Transports (WebSocket server,
//! wasm Worker) only move messages; all behavior lives here.

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

// Test support shared by the per-backend conformance runners.
#[doc(hidden)]
pub mod conformance;
mod content;
mod convert;
pub mod dto;
mod edit;
mod font;
pub mod job;
pub mod macros;
mod model;
pub mod protocol;
mod session;
pub mod source;
mod ts;

pub use dto::*;
pub use job::{Executor, QueueExecutor, Task, ThreadExecutor};
pub use macros::{MAX_MACRO_LOG, MacroHost, MacroOutput, MacroReporter, MacroRun};
pub use protocol::*;
pub use session::{GroupId, MAX_CHUNK, MAX_PAGE, SaveStream, Session, SubscriptionId};
pub use source::{NoFilesystem, Opened, SaveTarget, SourceProvider};
pub use ts::typescript;
