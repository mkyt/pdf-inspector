//! Long-running work. A job is a sequence of bounded steps so the same code
//! runs on a thread (native) and cooperatively between messages (wasm).

use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use pdfi_model::inventory::{self, Builder, Inventory, ObjClass};
use pdfi_schema::{Severity, Walk};
use pdfi_syntax::{ObjId, ObjLocation, Object};

use pdfi_edit::Saver;

use crate::convert;
use crate::dto::{DiagnosticInfo, JobInfo, JobKind, JobResult, JobState, SaveResult};
use crate::edit::DocFile;
use crate::font::{self, FontUsage};
use crate::protocol::Event;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, EventBus, lock};
use crate::source::{Opened, SaveTarget};

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

/// Runs each task on its own thread (native embedders).
pub struct ThreadExecutor;

impl Executor for ThreadExecutor {
    fn spawn(&self, mut task: Task) {
        std::thread::spawn(move || while task() {});
    }
}

impl<T: Executor + ?Sized> Executor for Arc<T> {
    fn spawn(&self, task: Task) {
        (**self).spawn(task)
    }
}

/// Objects processed per step.
const STEP: usize = 256;
/// Objects validated per step (each may load what it refers to).
const VALIDATE_STEP: usize = 32;
/// Pages scanned per inventory step.
const PAGE_STEP: usize = 32;
/// Pages a `FontUses` step goes through.
const FONT_PAGE_STEP: usize = 4;
/// Issues a validation job keeps for `JobDiagnostics`.
const MAX_KEPT: usize = 100_000;
/// Chunks (about 1 MB each) a save step writes.
const SAVE_CHUNKS: usize = 4;

pub(crate) struct Job {
    pub doc: Arc<Doc>,
    info: Mutex<JobInfo>,
    cancel: AtomicBool,
    work: Mutex<Work>,
    diags: Mutex<Vec<DiagnosticInfo>>,
    bus: Arc<EventBus>,
}

enum Work {
    Check(CheckWork),
    Validate(ValidateWork),
    Inventory(InventoryWork),
    FontUses(FontUsesWork),
    Save(SaveWork),
}

/// What a save job does with the file once written: reopen the document.
pub(crate) type Reopen = Box<dyn FnOnce(Opened) -> Result<(), ApiError> + Send>;

struct SaveWork {
    saver: Saver<DocFile>,
    /// `None` once committed (or failed).
    target: Option<Box<dyn SaveTarget>>,
    path: Option<String>,
    reopen: Option<Reopen>,
}

struct FontUsesWork {
    font: ObjId,
    pages: Vec<u32>,
    pos: usize,
    usage: FontUsage,
}

/// Scanning every object and page, then typing what the trailer reaches to
/// class the objects by schema type.
enum InventoryWork {
    Scan(Builder),
    Type {
        inv: Inventory,
        walk: Walk,
        /// Steps of the scan, before typing.
        scanned: u64,
    },
    Done,
}

struct ValidateWork {
    walk: Walk,
    /// Issues by severity: info, warning, error.
    counts: [u64; 3],
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
            JobKind::Validate { root } => {
                let ck = doc.schema.checker(&doc.file);
                let walk = match root {
                    None => Walk::document(&ck),
                    Some(r) => {
                        let id = (*r).into();
                        match ck.type_of(id) {
                            Some((ty, _)) => Walk::object(id, ty),
                            // Nothing to check against: finishes at once.
                            None => Walk::empty(),
                        }
                    }
                };
                Work::Validate(ValidateWork {
                    walk,
                    counts: [0; 3],
                })
            }
            JobKind::Inventory => {
                Work::Inventory(InventoryWork::Scan(Builder::new(&doc.model.on(&doc.file))))
            }
            JobKind::Save { .. } => unreachable!("save jobs are made by Job::save"),
            JobKind::FontUses { font } => Work::FontUses(FontUsesWork {
                font: (*font).into(),
                pages: font::pages_to_scan(&doc, (*font).into()),
                pos: 0,
                usage: FontUsage::default(),
            }),
        };
        let total = match &work {
            Work::Check(w) => w.ids.len() as u64,
            Work::Validate(w) => w.walk.pending() as u64,
            // Typing visits about as many objects as the scan.
            Work::Inventory(InventoryWork::Scan(b)) => {
                b.total() + doc.file.xref().live_ids().count() as u64
            }
            Work::Inventory(_) => 0,
            Work::FontUses(w) => w.pages.len() as u64,
            Work::Save(w) => w.saver.progress().1,
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
                error: None,
            }),
            doc,
            cancel: AtomicBool::new(false),
            work: Mutex::new(work),
            diags: Mutex::default(),
            bus,
        }
    }

    /// A job writing `doc` to `target`, then calling `reopen`.
    pub fn save(
        id: u32,
        doc: Arc<Doc>,
        kind: JobKind,
        saver: Saver<DocFile>,
        target: Box<dyn SaveTarget>,
        reopen: Reopen,
        bus: Arc<EventBus>,
    ) -> Self {
        let path = match &kind {
            JobKind::Save { path, .. } => path.clone(),
            _ => None,
        };
        let total = saver.progress().1;
        Self {
            info: Mutex::new(JobInfo {
                job: id,
                doc: doc.id,
                kind,
                state: JobState::Running,
                done: 0,
                total,
                result: None,
                error: None,
            }),
            doc,
            cancel: AtomicBool::new(false),
            work: Mutex::new(Work::Save(SaveWork {
                saver,
                target: Some(target),
                path,
                reopen: Some(reopen),
            })),
            diags: Mutex::default(),
            bus,
        }
    }

    pub fn info(&self) -> JobInfo {
        lock(&self.info).clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Issues found so far: total kept and the requested slice.
    pub fn diagnostics(&self, offset: u64, limit: u32) -> (u64, Vec<DiagnosticInfo>) {
        let d = lock(&self.diags);
        let o = (offset as usize).min(d.len());
        let n = (limit as usize).min(d.len() - o);
        (d.len() as u64, d[o..o + n].to_vec())
    }

    /// Do one bounded unit of work. Returns whether more remains.
    pub fn step(&self) -> bool {
        if lock(&self.info).state != JobState::Running {
            return false;
        }
        if self.cancel.load(Ordering::Relaxed) {
            // Drops a file being written.
            *lock(&self.work) = Work::Check(CheckWork::default());
            self.finish(JobState::Cancelled, None);
            return false;
        }
        let mut work = lock(&self.work);
        if let Work::Save(w) = &mut *work {
            return match save_step(w) {
                Ok(None) => {
                    let (done, total) = w.saver.progress();
                    drop(work);
                    {
                        let mut info = lock(&self.info);
                        info.done = done;
                        info.total = total;
                    }
                    self.bus.emit(&Event::JobProgress {
                        job: self.info().job,
                        doc: self.doc.id,
                        done,
                        total,
                    });
                    true
                }
                Ok(Some(r)) => {
                    drop(work);
                    {
                        let mut info = lock(&self.info);
                        info.done = r.bytes;
                        info.total = r.bytes;
                    }
                    self.finish(JobState::Completed, Some(JobResult::Save(r)));
                    false
                }
                Err(e) => {
                    w.target = None;
                    drop(work);
                    lock(&self.info).error = Some(e.message);
                    self.finish(JobState::Failed, None);
                    false
                }
            };
        }
        let (done, total, result) = match &mut *work {
            Work::Save(_) => unreachable!("handled above"),
            Work::Check(w) => {
                let (done, r) = self.check_step(w);
                (done, None, r)
            }
            Work::Validate(w) => {
                let (done, total, r) = self.validate_step(w);
                (done, Some(total), r)
            }
            Work::Inventory(w) => self.inventory_step(w),
            Work::FontUses(w) => {
                let (done, r) = self.font_uses_step(w);
                (done, None, r)
            }
        };
        drop(work);
        let total = {
            let mut info = lock(&self.info);
            info.done = done;
            if let Some(t) = total {
                info.total = t;
            }
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

    fn validate_step(&self, w: &mut ValidateWork) -> (u64, u64, Option<JobResult>) {
        let d = &self.doc;
        let ck = d.schema.checker(&d.file);
        let mut issues = Vec::new();
        let more = w.walk.step(&ck, VALIDATE_STEP, &mut issues);
        {
            let mut kept = lock(&self.diags);
            for i in &issues {
                w.counts[match i.severity {
                    Severity::Info => 0,
                    Severity::Warning => 1,
                    Severity::Error => 2,
                }] += 1;
                if kept.len() < MAX_KEPT {
                    // Spans of objects in object streams do not point into the file.
                    let in_file = i.obj.is_none_or(|id| {
                        d.file
                            .get(id)
                            .is_ok_and(|o| matches!(o.location, ObjLocation::File(_)))
                    });
                    kept.push(convert::issue(i, in_file));
                }
            }
        }
        let done = w.walk.done;
        let total = done + w.walk.pending() as u64;
        if more {
            return (done, total, None);
        }
        let kept = lock(&self.diags).len() as u64;
        let result = JobResult::Validate {
            objects: done,
            errors: w.counts[2],
            warnings: w.counts[1],
            infos: w.counts[0],
            kept,
        };
        (done, total, Some(result))
    }

    fn inventory_step(&self, w: &mut InventoryWork) -> (u64, Option<u64>, Option<JobResult>) {
        let d = &self.doc;
        let ck = d.schema.checker(&d.file);
        match w {
            InventoryWork::Scan(b) => {
                if b.step(&d.model.on(&d.file), STEP, PAGE_STEP) {
                    return (b.done(), None, None);
                }
                let scanned = b.done();
                let InventoryWork::Scan(b) = std::mem::replace(w, InventoryWork::Done) else {
                    unreachable!()
                };
                let inv = b.finish();
                let total = scanned + inv.objects;
                *w = InventoryWork::Type {
                    inv,
                    walk: Walk::types(&ck),
                    scanned,
                };
                (scanned, Some(total), None)
            }
            InventoryWork::Type { inv, walk, scanned } => {
                let mut none = Vec::new();
                let more = walk.step(&ck, VALIDATE_STEP, &mut none);
                // The trailer and unreachable objects make the count differ
                // from `inv.objects`: keep the bar moving forward.
                let typed = walk.done.min(inv.objects);
                if more {
                    return (*scanned + typed, None, None);
                }
                let done = *scanned + inv.objects;
                let InventoryWork::Type { mut inv, .. } = std::mem::replace(w, InventoryWork::Done)
                else {
                    unreachable!()
                };
                inv.classes = by_schema_type(std::mem::take(&mut inv.classes), |id| {
                    ck.type_of(id).map(|(t, _)| t.name())
                });
                let result = JobResult::Inventory(crate::dto::InventoryStats::of(&inv));
                *lock(&d.inventory) = Some(Arc::new(inv));
                (done, Some(done), Some(result))
            }
            InventoryWork::Done => (0, None, None),
        }
    }

    fn font_uses_step(&self, w: &mut FontUsesWork) -> (u64, Option<JobResult>) {
        let d = &self.doc;
        let end = (w.pos + FONT_PAGE_STEP).min(w.pages.len());
        for &p in &w.pages[w.pos..end] {
            w.usage.scan_page(d, w.font, p);
        }
        w.pos = end;
        let done = w.pos as u64;
        if w.pos < w.pages.len() {
            return (done, None);
        }
        let usage = std::mem::take(&mut w.usage);
        let result = JobResult::FontUses {
            scanned: usage.scanned.into(),
            pages: usage.pages.into(),
            uses: usage.uses.len() as u64,
        };
        lock(&d.font_uses).insert(w.font, Arc::new(usage));
        (done, Some(result))
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

/// Write a few chunks; at the end commit the file and reopen the document.
fn save_step(w: &mut SaveWork) -> Result<Option<SaveResult>, ApiError> {
    for _ in 0..SAVE_CHUNKS {
        let Some(chunk) = w.saver.next_chunk()? else {
            let target = w
                .target
                .take()
                .ok_or_else(|| ApiError::new(ErrorCode::Internal, "saved twice"))?;
            let opened = target.commit()?;
            let path = opened.path.clone().or_else(|| w.path.clone());
            if let Some(reopen) = w.reopen.take() {
                reopen(opened)?;
            }
            let r = w.saver.report();
            return Ok(Some(SaveResult {
                path,
                bytes: r.bytes,
                objects: r.objects,
                warnings: r.warnings.clone(),
            }));
        };
        let t = w
            .target
            .as_mut()
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "no file to write"))?;
        t.write(&chunk)?;
    }
    Ok(None)
}

/// Split classes by the schema type of their objects.
fn by_schema_type(
    classes: Vec<ObjClass>,
    type_of: impl Fn(ObjId) -> Option<String>,
) -> Vec<ObjClass> {
    let mut out = Vec::new();
    for c in classes {
        let mut split: std::collections::BTreeMap<Option<String>, Vec<ObjId>> = Default::default();
        for id in c.ids {
            split.entry(type_of(id)).or_default().push(id);
        }
        for (schema_type, ids) in split {
            out.push(ObjClass {
                kind: c.kind,
                type_name: c.type_name.clone(),
                subtype: c.subtype.clone(),
                schema_type,
                ids,
            });
        }
    }
    inventory::sort_classes(&mut out);
    out
}
