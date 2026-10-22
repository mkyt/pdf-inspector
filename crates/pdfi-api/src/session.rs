//! The session: open documents, jobs and the request dispatcher.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use pdfi_schema::{self as schema, Container, DocSchema, Origin};
use pdfi_syntax::{
    self as syn, ByteSource, DictExt, ObjLocation, Object, OpenOptions, PdfFile, Recovery,
};

use crate::convert::{self, value_of};
use crate::dto::*;
use crate::job::{Executor, Job};
use crate::protocol::*;
use crate::source::SourceProvider;

/// Largest binary chunk returned by one request.
pub const MAX_CHUNK: u32 = 4 << 20;
/// Largest number of items returned by one ranged request.
pub const MAX_PAGE: u32 = 2000;
/// Diagnostics attached to an [`ObjectView`] or [`ObjectSchema`].
const MAX_OBJECT_DIAGS: usize = 100;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Listener = Arc<dyn Fn(&Event) + Send + Sync>;

#[derive(Default)]
pub(crate) struct EventBus {
    next: AtomicU64,
    listeners: Mutex<Vec<(u64, Listener)>>,
}

impl EventBus {
    pub fn emit(&self, ev: &Event) {
        let ls: Vec<Listener> = lock(&self.listeners)
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for l in ls {
            l(ev);
        }
    }
}

pub(crate) struct Doc {
    pub id: DocId,
    pub info: DocInfo,
    pub file: PdfFile,
    /// Path and options it was opened with, to reuse it on a repeated open.
    key: Option<(String, Option<String>, bool)>,
    /// Object numbers present in the xref, ascending.
    nums: OnceLock<Vec<u32>>,
    pub schema: DocSchema,
}

impl Doc {
    fn nums(&self) -> &[u32] {
        self.nums
            .get_or_init(|| self.file.xref().iter().map(|(n, _)| n).collect())
    }
}

pub struct Session {
    provider: Box<dyn SourceProvider>,
    executor: Box<dyn Executor>,
    backend: String,
    docs: Mutex<BTreeMap<DocId, Arc<Doc>>>,
    jobs: Mutex<BTreeMap<JobId, Arc<Job>>>,
    next_doc: AtomicU32,
    next_job: AtomicU32,
    bus: Arc<EventBus>,
}

pub type SubscriptionId = u64;

impl Session {
    pub fn new(
        backend: impl Into<String>,
        provider: impl SourceProvider + 'static,
        executor: impl Executor + 'static,
    ) -> Self {
        Self {
            provider: Box::new(provider),
            executor: Box::new(executor),
            backend: backend.into(),
            docs: Mutex::default(),
            jobs: Mutex::default(),
            next_doc: AtomicU32::new(1),
            next_job: AtomicU32::new(1),
            bus: Arc::default(),
        }
    }

    /// Receive every event of this session until [`Session::unsubscribe`].
    pub fn subscribe(&self, f: impl Fn(&Event) + Send + Sync + 'static) -> SubscriptionId {
        let id = self.bus.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.bus.listeners).push((id, Arc::new(f)));
        id
    }

    pub fn unsubscribe(&self, id: SubscriptionId) {
        lock(&self.bus.listeners).retain(|(i, _)| *i != id);
    }

    pub fn dispatch(&self, req: Request) -> Reply {
        let mut blobs = Vec::new();
        let response = match self.handle(req, &mut blobs, true) {
            Ok(b) => Response::Ok(b),
            Err(e) => Response::Err(e),
        };
        Reply { response, blobs }
    }

    /// Dispatch a JSON-encoded [`ClientMessage`]. Malformed input yields an
    /// error response with id 0.
    pub fn dispatch_json(&self, msg: &str) -> (RequestId, Reply) {
        match serde_json::from_str::<ClientMessage>(msg) {
            Ok(m) => (m.id, self.dispatch(m.request)),
            Err(e) => (
                0,
                Reply {
                    response: Response::Err(ApiError::new(
                        ErrorCode::BadRequest,
                        format!("malformed message: {e}"),
                    )),
                    blobs: Vec::new(),
                },
            ),
        }
    }

    /// Open a document from a byte source (used by embedders that do not go
    /// through [`SourceProvider`], and by tests).
    pub fn open_source(
        &self,
        source: Arc<dyn ByteSource>,
        name: impl Into<String>,
        password: Option<String>,
        recover: bool,
    ) -> Result<DocSummary, ApiError> {
        self.open_inner(source, name.into(), None, password, recover)
    }

    fn open_inner(
        &self,
        source: Arc<dyn ByteSource>,
        name: String,
        path: Option<String>,
        password: Option<String>,
        recover: bool,
    ) -> Result<DocSummary, ApiError> {
        let size = source.len();
        let file = PdfFile::open(
            source,
            OpenOptions {
                password: password.as_ref().map(|p| p.as_bytes().to_vec()),
                recovery: if recover {
                    Recovery::Force
                } else {
                    Recovery::Auto
                },
                ..Default::default()
            },
        )?;
        let id = self.next_doc.fetch_add(1, Ordering::Relaxed);
        let doc = Arc::new(Doc {
            id,
            info: DocInfo {
                doc: id,
                name,
                path: path.clone(),
                size,
            },
            file,
            key: path.map(|p| (p, password, recover)),
            nums: OnceLock::new(),
            schema: DocSchema::new(),
        });
        lock(&self.docs).insert(id, doc.clone());
        self.bus.emit(&Event::DocOpened {
            info: doc.info.clone(),
        });
        Ok(summary(&doc))
    }

    fn doc(&self, id: DocId) -> Result<Arc<Doc>, ApiError> {
        lock(&self.docs)
            .get(&id)
            .cloned()
            .ok_or_else(|| ApiError::new(ErrorCode::NoSuchDoc, format!("no document {id}")))
    }

    fn job(&self, id: JobId) -> Result<Arc<Job>, ApiError> {
        lock(&self.jobs)
            .get(&id)
            .cloned()
            .ok_or_else(|| ApiError::new(ErrorCode::NoSuchJob, format!("no job {id}")))
    }

    fn handle(
        &self,
        req: Request,
        blobs: &mut Vec<Vec<u8>>,
        top: bool,
    ) -> Result<ResponseBody, ApiError> {
        match req {
            Request::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(ApiError::new(
                        ErrorCode::VersionMismatch,
                        format!(
                            "client speaks protocol {protocol}, backend speaks {PROTOCOL_VERSION}"
                        ),
                    ));
                }
                Ok(ResponseBody::Hello(HelloInfo {
                    protocol: PROTOCOL_VERSION,
                    backend: self.backend.clone(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    filesystem: self.provider.filesystem(),
                }))
            }
            Request::ListDir { path } => {
                Ok(ResponseBody::Dir(self.provider.list_dir(path.as_deref())?))
            }
            Request::Open {
                source,
                password,
                recover,
            } => {
                let opened = self.provider.open(&source)?;
                if let Some(p) = &opened.path {
                    let key = (p.clone(), password.clone(), recover);
                    let existing = lock(&self.docs)
                        .values()
                        .find(|d| d.key.as_ref() == Some(&key))
                        .cloned();
                    if let Some(d) = existing {
                        return Ok(ResponseBody::Doc(summary(&d)));
                    }
                }
                let s =
                    self.open_inner(opened.source, opened.name, opened.path, password, recover)?;
                Ok(ResponseBody::Doc(s))
            }
            Request::Docs => Ok(ResponseBody::Docs {
                docs: lock(&self.docs).values().map(|d| d.info.clone()).collect(),
            }),
            Request::Close { doc } => {
                let removed = lock(&self.docs).remove(&doc);
                if removed.is_none() {
                    return Err(ApiError::new(
                        ErrorCode::NoSuchDoc,
                        format!("no document {doc}"),
                    ));
                }
                for j in lock(&self.jobs).values() {
                    if j.doc.id == doc {
                        j.cancel();
                    }
                }
                self.bus.emit(&Event::DocClosed { doc });
                Ok(ResponseBody::Empty)
            }
            Request::Summary { doc } => Ok(ResponseBody::Doc(summary(&*self.doc(doc)?))),
            Request::Objects { doc, range } => {
                let d = self.doc(doc)?;
                Ok(ResponseBody::Objects(objects(&d, range)))
            }
            Request::Object { doc, id } => Ok(ResponseBody::Object(object(&*self.doc(doc)?, id)?)),
            Request::Children { doc, path, range } => Ok(ResponseBody::Children(children(
                &*self.doc(doc)?,
                &path,
                range,
            )?)),
            Request::Xref {
                doc,
                revision,
                range,
            } => Ok(ResponseBody::Xref(xref(&*self.doc(doc)?, revision, range)?)),
            Request::Schema { doc, root } => Ok(ResponseBody::Schema(object_schema(
                &*self.doc(doc)?,
                &root,
            )?)),
            Request::Diagnostics { doc, range } => {
                let d = self.doc(doc)?;
                let all = d.file.diagnostics();
                Ok(ResponseBody::Diagnostics(page(
                    all.len() as u64,
                    range,
                    |o, n| {
                        all.iter()
                            .skip(o)
                            .take(n)
                            .map(convert::diagnostic)
                            .collect()
                    },
                )))
            }
            Request::RawBytes { doc, offset, len } => {
                let d = self.doc(doc)?;
                let len = check_chunk(len)?;
                let total = d.file.source().len();
                let data = syn::source::read_vec(d.file.source(), offset, len as usize)?;
                Ok(push_blob(
                    blobs,
                    data,
                    BytesInfo {
                        part: 0,
                        offset,
                        len: 0,
                        total: Some(total),
                        eof: false,
                        stopped_at: None,
                        errors: Vec::new(),
                    },
                ))
            }
            Request::StreamData {
                doc,
                id,
                decoded,
                offset,
                len,
            } => {
                let d = self.doc(doc)?;
                let len = check_chunk(len)?;
                stream_data(&d, id, decoded, offset, len, blobs)
            }
            Request::StartJob { doc, job } => {
                let d = self.doc(doc)?;
                let id = self.next_job.fetch_add(1, Ordering::Relaxed);
                let j = Arc::new(Job::new(id, d, job, self.bus.clone()));
                lock(&self.jobs).insert(id, j.clone());
                let info = j.info();
                self.executor.spawn(Box::new(move || j.step()));
                Ok(ResponseBody::Job(info))
            }
            Request::JobStatus { job } => Ok(ResponseBody::Job(self.job(job)?.info())),
            Request::CancelJob { job } => {
                let j = self.job(job)?;
                j.cancel();
                Ok(ResponseBody::Job(j.info()))
            }
            Request::JobDiagnostics { job, range } => {
                let j = self.job(job)?;
                let (total, items) = j.diagnostics(range.offset, range.limit.min(MAX_PAGE));
                Ok(ResponseBody::Diagnostics(Page {
                    total,
                    offset: range.offset.min(total),
                    items,
                }))
            }
            // Cancellation of queued requests happens in the transport; by
            // the time a request reaches the session it has started.
            Request::Cancel { .. } => Ok(ResponseBody::Empty),
            Request::Batch { requests } => {
                if !top {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        "batches cannot be nested",
                    ));
                }
                let responses = requests
                    .into_iter()
                    .map(|r| match self.handle(r, blobs, false) {
                        Ok(b) => Response::Ok(b),
                        Err(e) => Response::Err(e),
                    })
                    .collect();
                Ok(ResponseBody::Batch { responses })
            }
        }
    }
}

fn check_chunk(len: u32) -> Result<u32, ApiError> {
    if len > MAX_CHUNK {
        return Err(ApiError::new(
            ErrorCode::TooLarge,
            format!("at most {MAX_CHUNK} bytes per request"),
        ));
    }
    Ok(len)
}

fn push_blob(blobs: &mut Vec<Vec<u8>>, data: Vec<u8>, mut info: BytesInfo) -> ResponseBody {
    info.part = blobs.len() as u32;
    info.len = data.len() as u64;
    if let Some(t) = info.total {
        info.eof = info.offset + info.len >= t;
    }
    blobs.push(data);
    ResponseBody::Bytes(info)
}

fn page<T>(total: u64, range: Range, items: impl FnOnce(usize, usize) -> Vec<T>) -> Page<T> {
    let offset = range.offset.min(total);
    let n = (total - offset).min(range.limit.min(MAX_PAGE) as u64) as usize;
    Page {
        total,
        offset,
        items: items(offset as usize, n),
    }
}

fn summary(d: &Doc) -> DocSummary {
    let f = &d.file;
    let trailer_ref = |k: &str| f.trailer().get_key(k).and_then(Object::as_ref_id);
    DocSummary {
        info: d.info.clone(),
        version: f.header().version.clone(),
        header_offset: f.header().offset,
        object_slots: f.xref().size(),
        xref_entries: d.nums().len() as u64,
        live_objects: f.xref().iter().filter(|(_, e)| e.is_live()).count() as u64,
        recovered: f.is_recovered(),
        revisions: f
            .revisions()
            .iter()
            .enumerate()
            .map(|(i, r)| RevisionInfo {
                index: i as u32,
                kind: convert::xref_kind(r.kind),
                offset: r.offset,
                span: r.span.into(),
                entries: (r.entries.len() + r.stream_entries.len()) as u64,
                xref_stream: r.xref_stream.map(Into::into),
            })
            .collect(),
        root: trailer_ref("Root").map(Into::into),
        info_dict: trailer_ref("Info").map(Into::into),
        encryption: f.security().map(|s| {
            let i = &s.info;
            EncryptionInfo {
                filter: convert::name_text(&i.filter),
                v: i.v,
                r: i.r,
                key_bits: i.key_bits as u32,
                stream_method: format!("{:?}", i.stream_method),
                string_method: format!("{:?}", i.string_method),
                permissions: i.permissions,
                authenticated: s.is_authenticated(),
            }
        }),
        diagnostics: f.diag_count() as u64,
        pdf_version: match d.schema.checker(f).version() {
            0 => None,
            v => Some(schema::version_text(v)),
        },
    }
}

fn objects(d: &Doc, range: Range) -> Page<ObjectSummary> {
    let nums = d.nums();
    page(nums.len() as u64, range, |o, n| {
        nums[o..o + n]
            .iter()
            .filter_map(|&num| {
                let e = d.file.xref().get(num)?;
                let mut s = ObjectSummary {
                    num,
                    entry: convert::xref_entry(e),
                    kind: None,
                    type_name: None,
                    subtype: None,
                    schema_type: None,
                    error: None,
                };
                if e.is_live() {
                    match d.file.get(syn::ObjId::new(num, e.generation())) {
                        Ok(o) => {
                            s.kind = Some(o.node.obj.type_name().into());
                            if let Some(dict) = o.node.obj.as_dict() {
                                s.type_name = dict.get_name("Type").map(convert::name_text);
                                s.subtype = dict.get_name("Subtype").map(convert::name_text);
                            }
                            s.schema_type = d
                                .schema
                                .checker(&d.file)
                                .type_of(o.id)
                                .map(|(t, _)| t.name());
                        }
                        Err(e) => s.error = Some(e.to_string()),
                    }
                }
                Some(s)
            })
            .collect()
    })
}

fn object(d: &Doc, id: ObjRef) -> Result<ObjectView, ApiError> {
    let o = d.file.get(id.into())?;
    let in_file = matches!(o.location, ObjLocation::File(_));
    let location = match o.location {
        ObjLocation::File(s) => Location::File { span: s.into() },
        ObjLocation::ObjStm { stream, index } => Location::ObjStm {
            stream: stream.into(),
            index,
        },
    };
    let stream = o.node.obj.as_stream().map(|s| {
        let specs = d.file.filter_specs(&s.dict);
        StreamInfo {
            data_span: match (&s.data, in_file) {
                (syn::StreamData::Source(sp), true) => Some((*sp).into()),
                _ => None,
            },
            raw_len: convert::raw_len(s),
            filters: specs.iter().map(|f| convert::name_text(&f.name)).collect(),
            encrypted: d.file.crypt_decoder(o.id, s, &specs).is_some(),
        }
    });
    let diagnostics = d
        .file
        .diagnostics()
        .iter()
        .filter(|x| x.obj == Some(o.id))
        .take(MAX_OBJECT_DIAGS)
        .map(convert::diagnostic)
        .collect();
    Ok(ObjectView {
        id: o.id.into(),
        location,
        value: value_of(&o.node.obj),
        span: o.node.span.filter(|_| in_file).map(Into::into),
        stream,
        diagnostics,
    })
}

fn children(d: &Doc, path: &NodePath, range: Range) -> Result<Page<NodeView>, ApiError> {
    let ck = d.schema.checker(&d.file);
    let held;
    let (root, mut ty, in_file) = match &path.root {
        NodeRoot::Trailer => (
            Container::Dict(d.file.trailer()),
            Some(ck.trailer_type()),
            true,
        ),
        NodeRoot::Object { id } => {
            held = d.file.get((*id).into())?;
            let c = Container::of(&held.node.obj)
                .ok_or_else(|| ApiError::new(ErrorCode::BadRequest, "not a container"))?;
            (
                c,
                ck.type_of(held.id).map(|(t, _)| t),
                matches!(held.location, ObjLocation::File(_)),
            )
        }
    };
    let mut cur = root;
    for &i in &path.path {
        let i = i as usize;
        let child_ty = ty.and_then(|t| ck.entry(t, cur, i, false).ty);
        let (_, n) = cur
            .child(i)
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "no such node"))?;
        cur = Container::of(&n.obj)
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "no such node"))?;
        ty = child_ty;
    }
    Ok(page(cur.len() as u64, range, |o, n| {
        (o..o + n)
            .filter_map(|i| {
                let (k, node) = cur.child(i)?;
                let schema = ty.map(|t| {
                    let e = ck.entry(t, cur, i, true);
                    let mut issues: Vec<DiagnosticInfo> = e
                        .issues
                        .iter()
                        .map(|x| convert::issue(x, in_file))
                        .collect();
                    // A direct dictionary or array: its missing keys and the like.
                    if let (Some(ct), Some(c)) = (e.ty, Container::of(&node.obj)) {
                        issues.extend(
                            ck.container_issues(ct, c)
                                .iter()
                                .map(|x| convert::issue(x, in_file)),
                        );
                    }
                    NodeSchema {
                        key: e.def.map(convert::key_doc),
                        value_type: e.ty.map(|t| t.name()),
                        issues,
                    }
                });
                Some(NodeView {
                    index: i as u32,
                    key: k.map(convert::name_text),
                    value: value_of(&node.obj),
                    span: node.span.filter(|_| in_file).map(Into::into),
                    schema,
                })
            })
            .collect()
    }))
}

/// Entries per container checked for [`ObjectSchema`]; the validation job
/// checks everything.
const SCHEMA_ENTRIES: usize = MAX_PAGE as usize;

fn object_schema(d: &Doc, root: &NodeRoot) -> Result<ObjectSchema, ApiError> {
    let ck = d.schema.checker(&d.file);
    let held;
    let (c, typed, in_file, obj) = match root {
        NodeRoot::Trailer => (
            Some(Container::Dict(d.file.trailer())),
            Some((ck.trailer_type(), Origin::Context)),
            true,
            None,
        ),
        NodeRoot::Object { id } => {
            held = d.file.get((*id).into())?;
            (
                Container::of(&held.node.obj),
                ck.type_of(held.id),
                matches!(held.location, ObjLocation::File(_)),
                Some(held.id),
            )
        }
    };
    let mut out = ObjectSchema {
        type_name: typed.map(|(t, _)| t.name()),
        origin: typed.map(|(_, o)| match o {
            Origin::Context => TypeOrigin::Context,
            Origin::Guess => TypeOrigin::Guess,
        }),
        note: typed
            .map(|(t, _)| t.note())
            .filter(|n| !n.is_empty())
            .map(Into::into),
        pdf_version: match ck.version() {
            0 => None,
            v => Some(schema::version_text(v)),
        },
        issues: Vec::new(),
        truncated: false,
    };
    if let (Some(c), Some((t, _))) = (c, typed) {
        let v = ck.validate(t, c, SCHEMA_ENTRIES);
        out.truncated = v.truncated || v.issues.len() > MAX_OBJECT_DIAGS;
        out.issues = v
            .issues
            .iter()
            .take(MAX_OBJECT_DIAGS)
            .map(|i| {
                let mut x = convert::issue(i, in_file);
                x.obj = obj.map(Into::into);
                x
            })
            .collect();
    }
    Ok(out)
}

fn xref(d: &Doc, revision: Option<u32>, range: Range) -> Result<Page<XrefRow>, ApiError> {
    let row = |(num, e): (u32, syn::xref::XrefEntry)| XrefRow {
        num,
        entry: convert::xref_entry(e),
    };
    match revision {
        None => {
            let nums = d.nums();
            Ok(page(nums.len() as u64, range, |o, n| {
                nums[o..o + n]
                    .iter()
                    .filter_map(|&num| d.file.xref().get(num).map(|e| row((num, e))))
                    .collect()
            }))
        }
        Some(i) => {
            let r = d
                .file
                .revisions()
                .get(i as usize)
                .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "no such revision"))?;
            let all = || r.entries.iter().chain(&r.stream_entries).copied();
            let total = (r.entries.len() + r.stream_entries.len()) as u64;
            Ok(page(total, range, |o, n| {
                all().skip(o).take(n).map(row).collect()
            }))
        }
    }
}

fn stream_data(
    d: &Doc,
    id: ObjRef,
    decoded: bool,
    offset: u64,
    len: u32,
    blobs: &mut Vec<Vec<u8>>,
) -> Result<ResponseBody, ApiError> {
    let o = d.file.get(id.into())?;
    let Object::Stream(s) = &o.node.obj else {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            format!("object {} is not a stream", o.id),
        ));
    };
    let mut info = BytesInfo {
        part: 0,
        offset,
        len: 0,
        total: None,
        eof: false,
        stopped_at: None,
        errors: Vec::new(),
    };
    if !decoded {
        info.total = Some(convert::raw_len(s));
        let mut r = d.file.raw_reader(s);
        io::copy(&mut (&mut r).take(offset), &mut io::sink())?;
        let mut data = Vec::new();
        r.take(len as u64).read_to_end(&mut data)?;
        return Ok(push_blob(blobs, data, info));
    }
    let (mut r, chain) = d.file.decoded_reader(o.id, s);
    info.stopped_at = chain.stopped_at.as_ref().map(convert::name_text);
    let mut data = Vec::new();
    // Decoding is sequential: skip to `offset`, then read one byte more than
    // asked to learn whether the data ends within this chunk.
    let res = io::copy(&mut (&mut r).take(offset), &mut io::sink()).and_then(|skipped| {
        if skipped < offset {
            return Ok(Some(skipped));
        }
        (&mut r).take(len as u64 + 1).read_to_end(&mut data)?;
        Ok((data.len() <= len as usize).then(|| offset + data.len() as u64))
    });
    match res {
        Ok(total) => info.total = total,
        Err(e) => {
            info.errors.push(e.to_string());
            info.total = Some(offset + data.len().min(len as usize) as u64);
        }
    }
    data.truncate(len as usize);
    info.offset = offset.min(info.total.unwrap_or(offset));
    info.errors.extend(r.errors().iter().cloned());
    if d.file.security().is_some_and(|s| !s.is_authenticated()) {
        info.errors
            .push("file is encrypted and no valid password was given".into());
    }
    Ok(push_blob(blobs, data, info))
}
