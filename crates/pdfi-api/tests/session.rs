use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pdfi_api::*;
use pdfi_syntax::MemSource;

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn session() -> (Session, Arc<QueueExecutor>) {
    let exec = Arc::new(QueueExecutor::default());
    (Session::new("test", NoFilesystem, exec.clone()), exec)
}

fn open(s: &Session, name: &str, password: Option<&str>) -> DocSummary {
    s.open_source(
        Arc::new(MemSource::new(fixture(name))),
        name,
        password.map(Into::into),
        false,
    )
    .unwrap()
}

fn ok(r: Reply) -> (ResponseBody, Vec<Vec<u8>>) {
    match r.response {
        Response::Ok(b) => (b, r.blobs),
        Response::Err(e) => panic!("{e}"),
    }
}

fn all() -> Range {
    Range {
        offset: 0,
        limit: 1000,
    }
}

#[test]
fn hello_checks_version() {
    let (s, _) = session();
    let (b, _) = ok(s.dispatch(Request::Hello {
        protocol: PROTOCOL_VERSION,
    }));
    assert!(matches!(b, ResponseBody::Hello(h) if !h.filesystem));
    let r = s.dispatch(Request::Hello { protocol: 999 });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::VersionMismatch));
}

#[test]
fn summary_and_objects() {
    let (s, _) = session();
    let sum = open(&s, "base.pdf", None);
    assert_eq!(sum.version.as_deref(), Some("1.7"));
    assert_eq!(
        sum.root,
        Some(ObjRef {
            num: 1,
            generation: 0
        })
    );
    assert_eq!(sum.revisions.len(), 1);
    let doc = sum.info.doc;

    let (b, _) = ok(s.dispatch(Request::Objects {
        doc,
        range: Range {
            offset: 1,
            limit: 3,
        },
    }));
    let ResponseBody::Objects(p) = b else {
        panic!()
    };
    assert_eq!(p.total, sum.xref_entries);
    assert_eq!(p.offset, 1);
    assert_eq!(p.items.len(), 3);
    assert_eq!(p.items[0].num, 1);
    assert_eq!(p.items[0].type_name.as_deref(), Some("Catalog"));

    // Past the end: empty page, offset clamped.
    let (b, _) = ok(s.dispatch(Request::Objects {
        doc,
        range: Range {
            offset: 10_000,
            limit: 10,
        },
    }));
    let ResponseBody::Objects(p) = b else {
        panic!()
    };
    assert!(p.items.is_empty());
    assert_eq!(p.offset, p.total);
}

#[test]
fn object_tree_walk() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;

    // Trailer -> /Root reference.
    let (b, _) = ok(s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Trailer,
            path: vec![],
        },
        range: all(),
    }));
    let ResponseBody::Children(p) = b else {
        panic!()
    };
    let root = p
        .items
        .iter()
        .find(|n| n.key.as_deref() == Some("Root"))
        .unwrap();
    let Value::Ref { id } = root.value else {
        panic!()
    };
    assert!(root.span.is_some());

    // Catalog -> /Pages -> 2 0 obj -> /Kids array.
    let (b, _) = ok(s.dispatch(Request::Object { doc, id }));
    let ResponseBody::Object(o) = b else { panic!() };
    assert!(matches!(&o.value, Value::Dict { type_name: Some(t), .. } if t == "Catalog"));
    assert!(matches!(o.location, Location::File { .. }));

    let pages = ObjRef {
        num: 2,
        generation: 0,
    };
    let (b, _) = ok(s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Object { id: pages },
            path: vec![],
        },
        range: all(),
    }));
    let ResponseBody::Children(p) = b else {
        panic!()
    };
    let kids = p
        .items
        .iter()
        .find(|n| n.key.as_deref() == Some("Kids"))
        .unwrap();
    assert!(matches!(kids.value, Value::Array { len: 3 }));

    let (b, _) = ok(s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Object { id: pages },
            path: vec![kids.index],
        },
        range: Range {
            offset: 1,
            limit: 1,
        },
    }));
    let ResponseBody::Children(p) = b else {
        panic!()
    };
    assert_eq!(p.total, 3);
    assert!(matches!(p.items[0].value, Value::Ref { id } if id.num == 4));

    // Bad path.
    let r = s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Object { id: pages },
            path: vec![99],
        },
        range: all(),
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::NotFound));
}

#[test]
fn strings_are_decoded() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;
    let (b, _) = ok(s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Object {
                id: ObjRef {
                    num: 7,
                    generation: 0,
                },
            },
            path: vec![],
        },
        range: all(),
    }));
    let ResponseBody::Children(p) = b else {
        panic!()
    };
    let text = |k: &str| match &p
        .items
        .iter()
        .find(|n| n.key.as_deref() == Some(k))
        .unwrap()
        .value
    {
        Value::String { text, .. } => text.clone(),
        v => panic!("{v:?}"),
    };
    assert_eq!(text("Title"), "Fixture (test)");
    assert_eq!(text("Author"), "AB");
}

fn stream_chunk(
    s: &Session,
    doc: DocId,
    num: u32,
    decoded: bool,
    offset: u64,
    len: u32,
) -> (BytesInfo, Vec<u8>) {
    let (b, mut blobs) = ok(s.dispatch(Request::StreamData {
        doc,
        id: ObjRef { num, generation: 0 },
        decoded,
        offset,
        len,
    }));
    let ResponseBody::Bytes(info) = b else {
        panic!()
    };
    assert_eq!(blobs.len(), 1);
    assert_eq!(info.len as usize, blobs[0].len());
    (info, blobs.remove(0))
}

#[test]
fn stream_data_in_chunks() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;
    let expected = b"BT /F1 24 Tf 72 700 Td (Page 1) Tj ET\n".repeat(20);

    let (info, whole) = stream_chunk(&s, doc, 10, true, 0, 1 << 20);
    assert_eq!(whole, expected);
    assert!(info.eof);
    assert_eq!(info.total, Some(expected.len() as u64));

    // Exactly the length: still known to end.
    let (info, _) = stream_chunk(&s, doc, 10, true, 0, expected.len() as u32);
    assert!(info.eof);

    let (info, part) = stream_chunk(&s, doc, 10, true, 100, 50);
    assert_eq!(part, &expected[100..150]);
    assert!(!info.eof);
    assert_eq!(info.total, None);

    let (info, part) = stream_chunk(&s, doc, 10, true, expected.len() as u64 + 5, 50);
    assert!(part.is_empty());
    assert!(info.eof);
    assert_eq!(info.offset, expected.len() as u64);

    // Raw data matches the bytes in the file at the stream's data span.
    let (b, _) = ok(s.dispatch(Request::Object {
        doc,
        id: ObjRef {
            num: 10,
            generation: 0,
        },
    }));
    let ResponseBody::Object(o) = b else { panic!() };
    let st = o.stream.unwrap();
    assert_eq!(st.filters, ["FlateDecode"]);
    let span = st.data_span.unwrap();
    let (info, raw) = stream_chunk(&s, doc, 10, false, 0, 1 << 20);
    assert_eq!(info.total, Some(st.raw_len));
    assert!(info.eof);
    let file = fixture("base.pdf");
    assert_eq!(raw, &file[span.start as usize..span.end as usize]);

    // Not a stream.
    let r = s.dispatch(Request::StreamData {
        doc,
        id: ObjRef {
            num: 1,
            generation: 0,
        },
        decoded: true,
        offset: 0,
        len: 10,
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::BadRequest));
}

#[test]
fn raw_bytes_and_limits() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;
    let file = fixture("base.pdf");
    let (b, blobs) = ok(s.dispatch(Request::RawBytes {
        doc,
        offset: 0,
        len: 8,
    }));
    assert_eq!(blobs[0], b"%PDF-1.7");
    assert!(matches!(b, ResponseBody::Bytes(i) if !i.eof && i.total == Some(file.len() as u64)));

    let (b, blobs) = ok(s.dispatch(Request::RawBytes {
        doc,
        offset: file.len() as u64 - 4,
        len: 100,
    }));
    assert_eq!(blobs[0], &file[file.len() - 4..]);
    assert!(matches!(b, ResponseBody::Bytes(i) if i.eof));

    let r = s.dispatch(Request::RawBytes {
        doc,
        offset: 0,
        len: MAX_CHUNK + 1,
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::TooLarge));
}

#[test]
fn objects_in_object_streams_have_no_file_spans() {
    let (s, _) = session();
    let doc = open(&s, "objstm.pdf", None).info.doc;
    let (b, _) = ok(s.dispatch(Request::Objects { doc, range: all() }));
    let ResponseBody::Objects(p) = b else {
        panic!()
    };
    let compressed = p
        .items
        .iter()
        .find(|o| matches!(o.entry, XrefEntry::Compressed { .. }))
        .unwrap();
    let id = ObjRef {
        num: compressed.num,
        generation: 0,
    };
    let (b, _) = ok(s.dispatch(Request::Object { doc, id }));
    let ResponseBody::Object(o) = b else { panic!() };
    assert!(matches!(o.location, Location::ObjStm { .. }));
    assert!(o.span.is_none());
    let (b, _) = ok(s.dispatch(Request::Children {
        doc,
        path: NodePath {
            root: NodeRoot::Object { id },
            path: vec![],
        },
        range: all(),
    }));
    let ResponseBody::Children(p) = b else {
        panic!()
    };
    assert!(p.items.iter().all(|n| n.span.is_none()));
}

#[test]
fn encrypted_and_revisions() {
    let (s, _) = session();
    let sum = open(&s, "aes-256.pdf", Some("user"));
    let enc = sum.encryption.unwrap();
    assert!(enc.authenticated);
    assert_eq!(enc.key_bits, 256);

    let sum = open(&s, "incremental.pdf", None);
    assert_eq!(sum.revisions.len(), 2);
    let (b, _) = ok(s.dispatch(Request::Xref {
        doc: sum.info.doc,
        revision: Some(0),
        range: all(),
    }));
    let ResponseBody::Xref(p) = b else { panic!() };
    assert_eq!(p.total, sum.revisions[0].entries);
}

#[test]
fn batch_collects_responses_and_blobs() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;
    let (b, blobs) = ok(s.dispatch(Request::Batch {
        requests: vec![
            Request::RawBytes {
                doc,
                offset: 0,
                len: 4,
            },
            Request::Summary { doc: 999 },
            Request::RawBytes {
                doc,
                offset: 4,
                len: 4,
            },
        ],
    }));
    let ResponseBody::Batch { responses } = b else {
        panic!()
    };
    assert_eq!(blobs, [b"%PDF".to_vec(), b"-1.7".to_vec()]);
    assert!(matches!(&responses[0], Response::Ok(ResponseBody::Bytes(i)) if i.part == 0));
    assert!(matches!(&responses[1], Response::Err(e) if e.code == ErrorCode::NoSuchDoc));
    assert!(matches!(&responses[2], Response::Ok(ResponseBody::Bytes(i)) if i.part == 1));

    let r = s.dispatch(Request::Batch {
        requests: vec![Request::Batch { requests: vec![] }],
    });
    let (ResponseBody::Batch { responses }, _) = ok(r) else {
        panic!()
    };
    assert!(matches!(&responses[0], Response::Err(e) if e.code == ErrorCode::BadRequest));
}

#[test]
fn jobs_report_progress_and_results() {
    let (s, exec) = session();
    let doc = open(&s, "broken.pdf", None).info.doc;
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sub = s.subscribe(move |e| ev.lock().unwrap().push(e.clone()));

    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::CheckObjects {
            decode_streams: true,
        },
    }));
    let ResponseBody::Job(info) = b else { panic!() };
    assert_eq!(info.state, JobState::Running);
    exec.run_all();

    let (b, _) = ok(s.dispatch(Request::JobStatus { job: info.job }));
    let ResponseBody::Job(done) = b else { panic!() };
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.done, done.total);
    let Some(JobResult::CheckObjects {
        ok: loaded,
        streams,
        ..
    }) = done.result
    else {
        panic!()
    };
    assert!(loaded > 0);
    assert_eq!(streams, 3);
    let evs = events.lock().unwrap();
    assert!(matches!(evs.last(), Some(Event::JobDone { info }) if info.job == done.job));
    drop(evs);

    // Cancelled before it runs.
    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::CheckObjects {
            decode_streams: false,
        },
    }));
    let ResponseBody::Job(info) = b else { panic!() };
    ok(s.dispatch(Request::CancelJob { job: info.job }));
    exec.run_all();
    let (b, _) = ok(s.dispatch(Request::JobStatus { job: info.job }));
    assert!(matches!(b, ResponseBody::Job(i) if i.state == JobState::Cancelled));

    s.unsubscribe(sub);
    let n = events.lock().unwrap().len();
    ok(s.dispatch(Request::Close { doc }));
    assert_eq!(events.lock().unwrap().len(), n);
    let r = s.dispatch(Request::Summary { doc });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::NoSuchDoc));
}

#[test]
fn json_round_trip() {
    let (s, _) = session();
    let doc = open(&s, "base.pdf", None).info.doc;
    let msg = format!(
        r#"{{"id":7,"request":{{"op":"objects","doc":{doc},"range":{{"offset":0,"limit":2}}}}}}"#
    );
    let (id, reply) = s.dispatch_json(&msg);
    assert_eq!(id, 7);
    let (_, out) = reply.into_frames(id);
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["kind"], "response");
    assert_eq!(json["response"]["ok"]["type"], "objects");
    assert_eq!(json["response"]["ok"]["items"][1]["entry"]["kind"], "inUse");

    let (id, reply) = s.dispatch_json("{nope");
    assert_eq!(id, 0);
    assert!(matches!(reply.response, Response::Err(e) if e.code == ErrorCode::BadRequest));

    let f = BinaryFrame {
        id: 42,
        part: 3,
        data: vec![1, 2, 3],
    };
    assert_eq!(BinaryFrame::decode(&f.encode()), Some(f));
}

#[test]
fn typescript_bindings_are_up_to_date() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/src/protocol/generated.ts");
    let ts = typescript();
    if std::env::var_os("UPDATE_TS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &ts).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == ts,
        "{} is stale; run `UPDATE_TS=1 cargo test -p pdfi-api`",
        path.display()
    );
}
