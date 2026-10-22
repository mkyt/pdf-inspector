//! Conformance runner calling the Rust API directly. This one defines
//! `tests/conformance/expected.json` (`UPDATE_CONFORMANCE=1`); the server
//! and wasm runners must match it.

use std::sync::{Arc, Mutex};

use pdfi_api::conformance::{self, Step};
use pdfi_api::*;
use pdfi_syntax::MemSource;

#[test]
fn conformance() {
    let exec = Arc::new(QueueExecutor::default());
    let s = Session::new("direct", NoFilesystem, exec.clone());
    let events: Arc<Mutex<Vec<Event>>> = Arc::default();
    {
        let events = events.clone();
        s.subscribe(move |ev| events.lock().unwrap().push(ev.clone()));
    }
    let mut out = Vec::new();
    for step in conformance::scenario() {
        let entry = match step {
            Step::Request(r) => {
                let r = s.dispatch(r);
                conformance::entry(&r.response, &r.blobs, &[])
            }
            Step::Open {
                file,
                password,
                recover,
            } => {
                let data = std::fs::read(conformance::fixtures().join(&file)).unwrap();
                let response =
                    match s.open_source(Arc::new(MemSource::new(data)), file, password, recover) {
                        Ok(sum) => Response::Ok(ResponseBody::Doc(sum)),
                        Err(e) => Response::Err(e),
                    };
                conformance::entry(&response, &[], &[])
            }
            Step::Job(r) => {
                let r = s.dispatch(r);
                let job = conformance::started_job(&r.response).expect("job started");
                events.lock().unwrap().clear();
                exec.run_all();
                let evs: Vec<Event> = events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| conformance::job_event(e, job).is_some())
                    .cloned()
                    .collect();
                conformance::entry(&r.response, &r.blobs, &evs)
            }
        };
        out.push(entry);
    }
    conformance::check("direct", out, true);
}
