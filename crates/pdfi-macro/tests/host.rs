//! Macros run by a session as jobs (`JobKind::Macro`, the server and
//! macOS), and the stored-macro requests.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pdfi_api::*;
use pdfi_macro::Library;
use pdfi_syntax::MemSource;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pdfi-macro-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Env {
    s: Arc<Session>,
    doc: DocId,
    events: Arc<Mutex<Vec<Event>>>,
}

fn env(dir: &std::path::Path) -> Env {
    let s = Arc::new(Session::new("test", NoFilesystem, ThreadExecutor));
    pdfi_macro::host::install(&s, Library::new(vec![dir.to_path_buf()]), "server");
    let events: Arc<Mutex<Vec<Event>>> = Arc::default();
    let e = events.clone();
    s.subscribe(move |ev| e.lock().unwrap().push(ev.clone()));
    let d = s
        .open_source(
            Arc::new(MemSource::new(fixture("model.pdf"))),
            "model.pdf",
            None,
            false,
        )
        .unwrap();
    Env {
        s,
        doc: d.info.doc,
        events,
    }
}

fn ok(r: Reply) -> ResponseBody {
    match r.response {
        Response::Ok(b) => b,
        Response::Err(e) => panic!("{e}"),
    }
}

impl Env {
    fn start(&self, name: &str, source: Option<&str>, params: &str) -> JobId {
        match ok(self.s.dispatch(Request::StartJob {
            doc: self.doc,
            job: JobKind::Macro {
                name: name.into(),
                source: source.map(str::to_string),
                params: params.into(),
            },
        })) {
            ResponseBody::Job(j) => j.job,
            b => panic!("{b:?}"),
        }
    }

    fn wait(&self, job: JobId) -> JobInfo {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let done = self.events.lock().unwrap().iter().find_map(|e| match e {
                Event::JobDone { info } if info.job == job => Some(info.clone()),
                _ => None,
            });
            if let Some(i) = done {
                return i;
            }
            assert!(Instant::now() < end, "job {job} did not end");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn logs(&self, job: JobId) -> Vec<(LogLevel, String)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::MacroLog {
                    job: j,
                    level,
                    text,
                } if *j == job => Some((*level, text.clone())),
                _ => None,
            })
            .collect()
    }

    fn summary(&self) -> DocSummary {
        match ok(self.s.dispatch(Request::Summary { doc: self.doc })) {
            ResponseBody::Doc(d) => d,
            b => panic!("{b:?}"),
        }
    }
}

const SCRIPT: &str = r#"
export const meta = {
  api: 1, name: "probe", title: "Probe", description: "test", edits: true,
  params: { n: { type: "integer", default: 2 } },
};
export default async function (doc, params, ctx) {
  ctx.log("pages", doc.pageCount);
  ctx.warn("careful");
  ctx.progress(1, 2, "half");
  await doc.rotatePages([0], 90);
  ctx.emit("out.txt", "hello", "text/plain");
  ctx.emit("data.json", { n: params.n, name: ctx.input.name, host: ctx.host });
  return [{ b: 1, a: 2 }];
}
"#;

#[test]
fn macro_job() {
    let dir = temp_dir("job");
    let e = env(&dir);
    let job = e.start("probe", Some(SCRIPT), r#"{"n": 5}"#);
    let info = e.wait(job);
    assert_eq!(info.state, JobState::Completed, "{:?}", info.error);
    let Some(JobResult::Macro(r)) = info.result else {
        panic!("{:?}", info.result)
    };
    assert_eq!(r.value, r#"[{"a":2,"b":1}]"#);
    assert_eq!(r.columns, Some(vec!["b".into(), "a".into()]));
    assert!(r.edits);
    assert_eq!(r.save, SaveMode::Incremental);
    assert_eq!(
        r.emitted
            .iter()
            .map(|f| (f.name.as_str(), f.size))
            .collect::<Vec<_>>()[0],
        ("out.txt", 5)
    );
    assert_eq!(
        e.logs(job),
        vec![
            (LogLevel::Info, "pages 5".to_string()),
            (LogLevel::Warn, "careful".to_string())
        ]
    );
    let progress = e.events.lock().unwrap().iter().any(|ev| {
        matches!(ev, Event::JobProgress { job: j, done: 1, total: 2, label: Some(l), .. } if *j == job && l == "half")
    });
    assert!(progress);
    // The files it wrote.
    let read = |index, offset, len| match ok(e.s.dispatch(Request::JobOutput {
        job,
        index,
        offset,
        len,
    })) {
        ResponseBody::Bytes(b) => b,
        b => panic!("{b:?}"),
    };
    let r0 = e.s.dispatch(Request::JobOutput {
        job,
        index: 0,
        offset: 1,
        len: 100,
    });
    assert_eq!(r0.blobs, vec![b"ello".to_vec()]);
    let b = read(0, 1, 100);
    assert!(b.eof);
    assert_eq!(b.total, Some(5));
    let r1 = e.s.dispatch(Request::JobOutput {
        job,
        index: 1,
        offset: 0,
        len: 1000,
    });
    let data: serde_json::Value = serde_json::from_slice(&r1.blobs[0]).unwrap();
    assert_eq!(
        data,
        serde_json::json!({ "n": 5, "name": "model.pdf", "host": "server" })
    );
    assert!(matches!(
        e.s.dispatch(Request::JobOutput {
            job,
            index: 2,
            offset: 0,
            len: 1
        })
        .response,
        Response::Err(ApiError {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    // One undoable step named after the macro.
    assert_eq!(e.summary().edit.undo.as_deref(), Some("Probe"));
}

#[test]
fn failing_and_cancelled_jobs_revert() {
    let dir = temp_dir("fail");
    let e = env(&dir);
    let failing = SCRIPT.replace("return [", "throw new Error(\"no\"); return [");
    let job = e.start("probe", Some(&failing), "{}");
    let info = e.wait(job);
    assert_eq!(info.state, JobState::Failed);
    assert!(info.error.unwrap().starts_with("no"));
    assert_eq!(e.summary().edit.undo, None);

    let bad = e.start("probe", Some(SCRIPT), r#"{"n": "x"}"#);
    let info = e.wait(bad);
    assert_eq!(info.state, JobState::Failed);
    assert!(info.error.unwrap().contains("parameter n"));

    let looping = SCRIPT.replace("return [", "for (;;) {} return [");
    let job = e.start("probe", Some(&looping), "{}");
    // Edits and undo wait while it runs.
    let end = Instant::now() + Duration::from_secs(10);
    while !e.summary().edit.dirty {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        e.s.dispatch(Request::Undo { doc: e.doc }).response,
        Response::Err(ApiError {
            code: ErrorCode::Busy,
            ..
        })
    ));
    ok(e.s.dispatch(Request::CancelJob { job }));
    let info = e.wait(job);
    assert_eq!(info.state, JobState::Cancelled);
    assert!(!e.summary().edit.dirty);
}

#[test]
fn stored_macros() {
    let dir = temp_dir("store");
    let e = env(&dir);
    let (items, d) = match ok(e.s.dispatch(Request::Macros)) {
        ResponseBody::Macros { items, dir } => (items, dir),
        b => panic!("{b:?}"),
    };
    assert_eq!(d, Some(dir.display().to_string()));
    let fonts = items.iter().find(|m| m.name == "fonts").unwrap();
    assert!(fonts.bundled && fonts.meta.is_some() && !fonts.shadowed);

    let saved = match ok(e.s.dispatch(Request::SaveMacro {
        name: "probe-file".into(),
        code: SCRIPT.into(),
    })) {
        ResponseBody::Macro(m) => m,
        b => panic!("{b:?}"),
    };
    assert_eq!(
        (saved.name.as_str(), saved.stem.as_str()),
        ("probe", "probe-file")
    );
    assert!(dir.join("probe-file.js").is_file());
    // A broken one is stored and listed with its error.
    ok(e.s.dispatch(Request::SaveMacro {
        name: "broken".into(),
        code: "export const meta = ;".into(),
    }));
    let items = match ok(e.s.dispatch(Request::Macros)) {
        ResponseBody::Macros { items, .. } => items,
        b => panic!("{b:?}"),
    };
    assert!(
        items
            .iter()
            .any(|m| m.stem == "broken" && m.error.is_some())
    );
    match ok(e.s.dispatch(Request::MacroSource {
        name: "probe".into(),
    })) {
        ResponseBody::MacroSource { info, code } => {
            assert_eq!(code, SCRIPT);
            assert!(!info.bundled);
        }
        b => panic!("{b:?}"),
    }
    // Run it by name.
    let job = e.start("probe", None, "{}");
    assert_eq!(e.wait(job).state, JobState::Completed);
    // Names are file names, never paths.
    for bad in ["../x", "a/b", ".hidden", ""] {
        assert!(
            matches!(
                e.s.dispatch(Request::SaveMacro {
                    name: bad.into(),
                    code: String::new()
                })
                .response,
                Response::Err(ApiError {
                    code: ErrorCode::BadRequest,
                    ..
                })
            ),
            "{bad}"
        );
    }
    let job = e.start("/etc/passwd.js", None, "{}");
    assert_eq!(e.wait(job).state, JobState::Failed);
    ok(e.s.dispatch(Request::DeleteMacro {
        name: "probe-file".into(),
    }));
    assert!(!dir.join("probe-file.js").exists());
    assert!(matches!(
        e.s.dispatch(Request::DeleteMacro {
            name: "fonts".into()
        })
        .response,
        Response::Err(ApiError {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_macro_host() {
    let s = Session::new("test", NoFilesystem, ThreadExecutor);
    assert!(matches!(
        s.dispatch(Request::Macros).response,
        Response::Err(ApiError {
            code: ErrorCode::Unsupported,
            ..
        })
    ));
}
