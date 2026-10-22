//! Long-running work. A job is a sequence of bounded steps so the same code
//! runs on a thread (native) and cooperatively between messages (wasm).

use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pdfi_syntax::{ObjId, Object};

use crate::dto::{JobInfo, JobKind, JobResult, JobState};
use crate::protocol::Event;
use crate::session::{Doc, EventBus, lock};

pub type Task = Box<dyn FnMut() -> bool + Send>;

/// Runs job tasks. A task is called repeatedly until it returns `false`;
/// each call performs a bounded amount of work.
pub trait Executor: Send + Sync {
    fn spawn(&self, task: Task);
}

/// Cooperative executor: the embedder calls [`QueueExecutor::run_step`] when
/// idle (wasm) or [`QueueExecutor::run_all`] (tests).
#[derive(Default)]
pub struct QueueExecutor {
    tasks: Mutex<VecDeque<Task>>,
}

impl QueueExecutor {
    /// Run one step of the next task. Returns whether tasks remain.
    pub fn run_step(&self) -> bool {
        let Some(mut t) = lock(&self.tasks).pop_front() else {
            return false;
        };
        let more = t();
        let mut q = lock(&self.tasks);
        if more {
            q.push_back(t);
        }
        !q.is_empty()
    }

    pub fn run_all(&self) {
        while self.run_step() {}
    }
}

impl Executor for QueueExecutor {
    fn spawn(&self, task: Task) {
        lock(&self.tasks).push_back(task);
    }
}

impl<T: Executor + ?Sized> Executor for Arc<T> {
    fn spawn(&self, task: Task) {
        (**self).spawn(task)
    }
}

/// Objects processed per step.
const STEP: usize = 256;

pub(crate) struct Job {
    pub doc: Arc<Doc>,
    info: Mutex<JobInfo>,
    cancel: AtomicBool,
    work: Mutex<Work>,
    bus: Arc<EventBus>,
}

enum Work {
    Check(CheckWork),
}

#[derive(Default)]
struct CheckWork {
    decode: bool,
    ids: Vec<ObjId>,
    pos: usize,
    diags_before: usize,
    ok: u64,
    failed: u64,
    missing: u64,
    streams: u64,
    decoded_bytes: u64,
    decode_errors: u64,
}

impl Job {
    pub fn new(id: u32, doc: Arc<Doc>, kind: JobKind, bus: Arc<EventBus>) -> Self {
        let work = match &kind {
            JobKind::CheckObjects { decode_streams } => Work::Check(CheckWork {
                decode: *decode_streams,
                ids: doc.file.xref().live_ids().collect(),
                diags_before: doc.file.diag_count(),
                ..Default::default()
            }),
        };
        let total = match &work {
            Work::Check(w) => w.ids.len() as u64,
        };
        Self {
            info: Mutex::new(JobInfo {
                job: id,
                doc: doc.id,
                kind,
                state: JobState::Running,
                done: 0,
                total,
                result: None,
            }),
            doc,
            cancel: AtomicBool::new(false),
            work: Mutex::new(work),
            bus,
        }
    }

    pub fn info(&self) -> JobInfo {
        lock(&self.info).clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Do one bounded unit of work. Returns whether more remains.
    pub fn step(&self) -> bool {
        if lock(&self.info).state != JobState::Running {
            return false;
        }
        if self.cancel.load(Ordering::Relaxed) {
            self.finish(JobState::Cancelled, None);
            return false;
        }
        let mut work = lock(&self.work);
        let (done, result) = match &mut *work {
            Work::Check(w) => self.check_step(w),
        };
        drop(work);
        let total = {
            let mut info = lock(&self.info);
            info.done = done;
            info.total
        };
        match result {
            Some(r) => {
                self.finish(JobState::Completed, Some(r));
                false
            }
            None => {
                self.bus.emit(&Event::JobProgress {
                    job: self.info().job,
                    doc: self.doc.id,
                    done,
                    total,
                });
                true
            }
        }
    }

    fn finish(&self, state: JobState, result: Option<JobResult>) {
        let info = {
            let mut info = lock(&self.info);
            info.state = state;
            info.result = result;
            info.clone()
        };
        self.bus.emit(&Event::JobDone { info });
    }

    fn check_step(&self, w: &mut CheckWork) -> (u64, Option<JobResult>) {
        let file = &self.doc.file;
        let end = (w.pos + STEP).min(w.ids.len());
        for &id in &w.ids[w.pos..end] {
            match file.get(id) {
                Ok(o) => {
                    w.ok += 1;
                    if let (Object::Stream(s), true) = (&o.node.obj, w.decode) {
                        w.streams += 1;
                        let (mut r, _) = file.decoded_reader(o.id, s);
                        let mut buf = vec![0; 1 << 16];
                        loop {
                            match r.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => w.decoded_bytes += n as u64,
                                Err(_) => {
                                    w.decode_errors += 1;
                                    break;
                                }
                            }
                        }
                        if !r.errors().is_empty() {
                            w.decode_errors += 1;
                        }
                    }
                }
                Err(pdfi_syntax::Error::Missing(_)) => w.missing += 1,
                Err(_) => w.failed += 1,
            }
        }
        w.pos = end;
        let done = w.pos as u64;
        if w.pos < w.ids.len() {
            return (done, None);
        }
        let result = JobResult::CheckObjects {
            ok: w.ok,
            failed: w.failed,
            missing: w.missing,
            streams: w.streams,
            decoded_bytes: w.decoded_bytes,
            decode_errors: w.decode_errors,
            diagnostics: file.diag_count().saturating_sub(w.diags_before) as u64,
        };
        (done, Some(result))
    }
}
