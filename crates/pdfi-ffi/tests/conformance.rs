//! Conformance runner through the FFI surface: documents opened by path,
//! jobs on threads, events through an `EventListener`. Must match
//! `tests/conformance/expected.json`.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pdfi_api::conformance::{self, Step};
use pdfi_api::{ApiError, Event, OpenSource, Request, Response};
use pdfi_ffi::{CancelToken, EventListener, PdfSession, PdfiError, Reply};

#[derive(Default)]
struct Events {
    list: Mutex<Vec<Event>>,
    changed: Condvar,
}

impl EventListener for Events {
    fn on_event(&self, event: Event) {
        self.list.lock().unwrap().push(event);
        self.changed.notify_all();
    }
}

impl Events {
    /// The events of `job` up to its `jobDone`.
    fn wait_job(&self, job: u32) -> Vec<Event> {
        let mut list = self.list.lock().unwrap();
        loop {
            let mine: Vec<Event> = list
                .iter()
                .filter(|e| conformance::job_event(e, job).is_some())
                .cloned()
                .collect();
            if mine
                .last()
                .is_some_and(|e| conformance::job_event(e, job) == Some(true))
            {
                return mine;
            }
            let (l, timeout) = self
                .changed
                .wait_timeout(list, Duration::from_secs(30))
                .unwrap();
            assert!(!timeout.timed_out(), "job {job} did not finish");
            list = l;
        }
    }
}

fn response(r: &Result<Reply, PdfiError>) -> (Response, &[Vec<u8>]) {
    match r {
        Ok(r) => (Response::Ok(r.body.clone()), &r.blobs),
        Err(PdfiError::Api { code, message }) => {
            (Response::Err(ApiError::new(*code, message.clone())), &[])
        }
    }
}

#[test]
fn conformance() {
    let s = PdfSession::new();
    let events = Arc::new(Events::default());
    s.subscribe(events.clone());
    let mut out = Vec::new();
    for step in conformance::scenario() {
        let entry = match step {
            Step::Request(r) => {
                let r = s.dispatch(r);
                let (resp, blobs) = response(&r);
                conformance::entry(&resp, blobs, &[])
            }
            Step::Open {
                file,
                password,
                recover,
            } => {
                let path = conformance::fixtures().join(&file);
                let r = s.dispatch(Request::Open {
                    source: OpenSource::Path {
                        path: path.to_string_lossy().into_owned(),
                    },
                    password,
                    recover,
                });
                let (resp, blobs) = response(&r);
                conformance::entry(&resp, blobs, &[])
            }
            Step::Job(r) => {
                events.list.lock().unwrap().clear();
                let r = s.dispatch_cancellable(r, CancelToken::new());
                let (resp, blobs) = response(&r);
                let job = conformance::started_job(&resp).expect("job started");
                let evs = events.wait_job(job);
                conformance::entry(&resp, blobs, &evs)
            }
        };
        out.push(entry);
    }
    conformance::check("ffi", out, false);
}
