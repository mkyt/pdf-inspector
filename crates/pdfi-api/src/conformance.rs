//! Protocol conformance (docs/DESIGN.md §7): one scenario, run by every
//! backend (Rust direct, WebSocket server, wasm), must produce the same
//! transcript, `tests/conformance/expected.json`.
//!
//! A runner walks [`scenario`], performs each [`Step`] its own way, turns
//! the result into an [`entry`] and finally calls [`check`]. The wasm runner
//! (`apps/web/scripts/conformance.ts`) mirrors [`entry`] in TypeScript.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::dto::JobId;
use crate::protocol::{Event, Request, Response};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    /// Send as is.
    Request(Request),
    /// Open `tests/fixtures/<file>` the backend's own way; answer like
    /// `Request::Open`.
    Open {
        file: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        recover: bool,
    },
    /// Send a `startJob`, then also record the job's events up to `jobDone`.
    Job(Request),
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

pub fn scenario() -> Vec<Step> {
    #[derive(Deserialize)]
    struct File {
        steps: Vec<Step>,
    }
    let p = dir().join("scenario.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str::<File>(&text)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .steps
}

/// The job a `startJob` response refers to.
pub fn started_job(r: &Response) -> Option<JobId> {
    match r {
        Response::Ok(crate::protocol::ResponseBody::Job(j)) => Some(j.job),
        _ => None,
    }
}

/// Whether `ev` belongs to `job`, and whether it is the last one.
pub fn job_event(ev: &Event, job: JobId) -> Option<bool> {
    match ev {
        Event::JobProgress { job: j, .. } if *j == job => Some(false),
        Event::JobDone { info } if info.job == job => Some(true),
        _ => None,
    }
}

/// One transcript entry, with what legitimately differs between backends
/// masked: the backend name and filesystem flag in `hello`, and file paths.
pub fn entry(response: &Response, blobs: &[Vec<u8>], events: &[Event]) -> Value {
    let mut e = json!({
        "response": response,
        "blobs": blobs.iter().map(|b| hex(b)).collect::<Vec<_>>(),
    });
    if !events.is_empty() {
        e["events"] = json!(events);
    }
    normalize(&mut e);
    e
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn normalize(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if m.get("type").and_then(Value::as_str) == Some("hello") {
                m.insert("backend".into(), Value::Null);
                m.insert("filesystem".into(), Value::Null);
            }
            for (k, x) in m.iter_mut() {
                if k == "path" && x.is_string() {
                    *x = Value::Null;
                } else {
                    normalize(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalize),
        _ => {}
    }
}

/// Compare with `expected.json`; with `UPDATE_CONFORMANCE=1` (Rust direct
/// runner only) rewrite it instead.
pub fn check(runner: &str, entries: Vec<Value>, may_update: bool) {
    let p = dir().join("expected.json");
    let actual = Value::Array(entries);
    if may_update && std::env::var_os("UPDATE_CONFORMANCE").is_some() {
        let text = serde_json::to_string_pretty(&actual).expect("json") + "\n";
        std::fs::write(&p, text).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        return;
    }
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let expected: Value = serde_json::from_str(&text).expect("expected.json");
    let (Value::Array(a), Value::Array(e)) = (&actual, &expected) else {
        unreachable!()
    };
    for (i, (x, y)) in a.iter().zip(e).enumerate() {
        assert!(
            x == y,
            "{runner}: step {i} differs from expected.json\n  actual:   {x}\n  expected: {y}\n\
             (UPDATE_CONFORMANCE=1 cargo test -p pdfi-api --test conformance regenerates it)"
        );
    }
    assert_eq!(a.len(), e.len(), "{runner}: number of steps differs");
}
