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
        filter: None,
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
        filter: None,
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
    let (b, _) = ok(s.dispatch(Request::Objects {
        doc,
        range: all(),
        filter: None,
    }));
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

fn decode_image(s: &Session, doc: DocId, num: u32, reduce: u32) -> (DecodedImage, Vec<u8>) {
    let (b, mut blobs) = ok(s.dispatch(Request::DecodeImage {
        doc,
        id: ObjRef { num, generation: 0 },
        reduce,
    }));
    let ResponseBody::DecodedImage(img) = b else {
        panic!()
    };
    let data = img.part.map(|p| std::mem::take(&mut blobs[p as usize]));
    (img, data.unwrap_or_default())
}

#[test]
fn decoded_images_and_stored_files() {
    let (s, _) = session();
    let doc = open(&s, "paint.pdf", None).info.doc;

    // A one-channel JPEG of ink amounts (/Separation /Black): no ink |
    // full ink. Saved as is, viewers would show it inverted.
    let (img, px) = decode_image(&s, doc, 69, 1);
    assert_eq!((img.width, img.height, img.file), (16, 8, None));
    assert_eq!(px.len(), 16 * 8 * 4);
    assert!(px[..3].iter().all(|&c| c > 240), "{:?}", &px[..4]);
    assert!(px[60..63].iter().all(|&c| c < 64), "{:?}", &px[60..64]);
    assert_eq!(px[3], 255);

    // Adobe CMYK JPEG: viewers invert it, PDF readers follow /Decode.
    assert_eq!(decode_image(&s, doc, 68, 1).0.file, None);
    assert_eq!(
        decode_image(&s, doc, 60, 1).0.file,
        Some(ImageFile::Jpeg2000)
    );
    // Reduced while decoding.
    let (img, px) = decode_image(&s, doc, 65, 4);
    assert_eq!((img.width, img.height, px.len()), (16, 16, 16 * 16 * 4));

    let doc = open(&s, "render.pdf", None).info.doc;
    assert_eq!(decode_image(&s, doc, 27, 1).0.file, Some(ImageFile::Jpeg));
    assert_eq!(decode_image(&s, doc, 28, 1).0.file, Some(ImageFile::Jpeg));
    assert_eq!(decode_image(&s, doc, 29, 1).0.file, None);
    // Stencil masks paint black where the sample is 0.
    let (img, px) = decode_image(&s, doc, 23, 1);
    assert_eq!((img.width, img.height, img.file), (8, 8, None));
    assert_eq!(&px[..4], [0, 0, 0, 0]);
    assert_eq!(&px[4..8], [0, 0, 0, 255]);

    // Not an image.
    let r = s.dispatch(Request::DecodeImage {
        doc,
        id: ObjRef {
            num: 1,
            generation: 0,
        },
        reduce: 1,
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::BadRequest));
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

#[test]
fn object_classes_filters_and_ref_paths() {
    let (s, exec) = session();
    let doc = open(&s, "model.pdf", None).info.doc;
    let r = s.dispatch(Request::ObjectClasses { doc });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::NotReady));
    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::Inventory,
    }));
    assert!(matches!(b, ResponseBody::Job(_)));
    exec.run_all();

    let (b, _) = ok(s.dispatch(Request::ObjectClasses { doc }));
    let ResponseBody::ObjectClasses { classes } = b else {
        panic!()
    };
    let page = classes
        .iter()
        .find(|c| c.type_name.as_deref() == Some("Page"))
        .expect("a /Page class")
        .clone();

    let (b, _) = ok(s.dispatch(Request::Objects {
        doc,
        range: all(),
        filter: Some(ObjectFilter {
            classes: Some(vec![page.clone()]),
            unreferenced: false,
        }),
    }));
    let ResponseBody::Objects(p) = b else {
        panic!()
    };
    assert_eq!(p.total, page.count);
    assert!(
        p.items
            .iter()
            .all(|o| o.type_name.as_deref() == Some("Page"))
    );
    assert!(p.items.windows(2).all(|w| w[0].num < w[1].num));

    // Reached through the page tree, not through back links.
    let first = p.items[0].num;
    let id = ObjRef {
        num: first,
        generation: 0,
    };
    let (b, _) = ok(s.dispatch(Request::RefPath { doc, id }));
    let ResponseBody::RefPath(path) = b else {
        panic!()
    };
    assert!(path.reachable);
    assert_eq!(path.steps[0].from, NodeRoot::Trailer);
    assert_eq!(path.steps[0].key, "/Root");
    assert_eq!(path.steps[1].key, "/Pages");
    assert!(path.steps.last().unwrap().key.starts_with("/Kids["));
}

#[test]
fn annotation_layer_picks_appearances() {
    let (s, _) = session();
    let doc = open(&s, "annots.pdf", None).info.doc;
    let pixel = |pick: Vec<AppearancePick>, x: usize, y: usize| {
        let (b, blobs) = ok(s.dispatch(Request::Render {
            doc,
            content: ContentRef::Page { index: 0 },
            scale: 1.0,
            tile: None,
            until: None,
            layer: RenderLayer::Annots { only: None, pick },
        }));
        let ResponseBody::Bitmap(info) = b else {
            panic!("{b:?}")
        };
        let i = (y * info.rect.width as usize + x) * 4;
        blobs[info.part as usize][i..i + 4].to_vec()
    };
    let pick = |index, kind: &str, state: Option<&str>| AppearancePick {
        index,
        kind: kind.into(),
        state: state.map(Into::into),
    };
    // The square's rollover, the checkbox pressed, and its other state.
    assert_eq!(pixel(vec![], 100, 150), [255, 0, 0, 255]);
    assert_eq!(
        pixel(vec![pick(2, "R", None)], 100, 150),
        [255, 128, 0, 255]
    );
    assert_eq!(
        pixel(vec![pick(4, "D", Some("On"))], 320, 180),
        [0, 102, 0, 255]
    );
    assert_eq!(pixel(vec![pick(4, "N", Some("Off"))], 320, 180)[3], 0);
    // States without /AS: not painted, unless one is picked.
    assert_eq!(pixel(vec![], 360, 40)[3], 0);
    assert_eq!(
        pixel(vec![pick(7, "N", Some("On"))], 360, 40),
        [0, 255, 0, 255]
    );
}

#[test]
fn program_glyphs_list_every_glyph_with_its_codes() {
    let (s, _) = session();
    let doc = open(&s, "fonts.pdf", None).info.doc;
    // PdfiCFF: /Differences sends 42 to A too, so glyph 2 has no code.
    let (b, _) = ok(s.dispatch(Request::ProgramGlyphs {
        doc,
        id: ObjRef {
            num: 23,
            generation: 0,
        },
        range: all(),
    }));
    let ResponseBody::ProgramGlyphs(p) = b else {
        panic!("{b:?}")
    };
    assert_eq!(p.total, 3);
    let codes: Vec<_> = p.items.iter().map(|g| g.codes.clone()).collect();
    assert_eq!(codes, [vec![], vec!["41".to_string(), "42".into()], vec![]]);
    assert_eq!(p.items[1].text.as_deref(), Some("A"));
    assert!(p.items[2].outline.is_some());
    assert!(p.items[2].width > 0.0);
    // Not embedded: nothing to list.
    let (b, _) = ok(s.dispatch(Request::ProgramGlyphs {
        doc,
        id: ObjRef {
            num: 28,
            generation: 0,
        },
        range: all(),
    }));
    let ResponseBody::ProgramGlyphs(p) = b else {
        panic!("{b:?}")
    };
    assert_eq!(p.total, 0);
}

#[test]
fn font_uses_find_text_on_pages_and_in_forms() {
    let (s, exec) = session();
    let doc = open(&s, "content.pdf", None).info.doc;
    let id = ObjRef {
        num: 20,
        generation: 0,
    };
    let uses = || {
        let (b, _) = ok(s.dispatch(Request::FontUses {
            doc,
            id,
            range: all(),
        }));
        let ResponseBody::FontUses(u) = b else {
            panic!("{b:?}")
        };
        u
    };
    assert!(!uses().scanned);

    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::FontUses { font: id },
    }));
    let ResponseBody::Job(info) = b else { panic!() };
    exec.run_all();
    let (b, _) = ok(s.dispatch(Request::JobStatus { job: info.job }));
    let ResponseBody::Job(done) = b else { panic!() };
    assert_eq!(
        done.result,
        Some(JobResult::FontUses {
            scanned: 3,
            pages: 1,
            uses: 3
        })
    );

    // Page 2 has the font in its resources but shows nothing with it.
    let u = uses();
    assert!(u.scanned);
    assert_eq!((u.pages_scanned, u.pages), (3, 1));
    let texts: Vec<_> = u.uses.items.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(texts, ["Hello", "W orld", "in form"]);
    // Text in a form: at the page's Do, with the form's own operation.
    let f = &u.uses.items[2];
    assert_eq!(
        f.form,
        Some(ObjRef {
            num: 31,
            generation: 0
        })
    );
    assert_eq!(f.form_op, Some(3));
    assert!(f.bbox.is_some());
    let l = u.codes.iter().find(|c| c.bytes == "6C").unwrap();
    assert_eq!(l.count, 3);
}

/// Saves into memory; a committed file can be opened again.
#[derive(Clone, Default)]
struct MemFiles(Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>);

struct MemTarget {
    files: MemFiles,
    path: String,
    data: Vec<u8>,
}

impl SaveTarget for MemTarget {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.data.extend_from_slice(data);
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<Opened, ApiError> {
        let data = self.data;
        self.files
            .0
            .lock()
            .unwrap()
            .insert(self.path.clone(), data.clone());
        Ok(Opened {
            source: Arc::new(MemSource::new(data)),
            name: self.path.clone(),
            path: Some(self.path),
        })
    }
}

impl SourceProvider for MemFiles {
    fn filesystem(&self) -> bool {
        true
    }

    fn open(&self, source: &OpenSource) -> Result<Opened, ApiError> {
        let OpenSource::Path { path } = source;
        let data = match self.0.lock().unwrap().get(path) {
            Some(d) => d.clone(),
            None => fixture(path),
        };
        Ok(Opened {
            source: Arc::new(MemSource::new(data)),
            name: path.clone(),
            path: Some(path.clone()),
        })
    }

    fn create(&self, path: &str) -> Result<Box<dyn SaveTarget>, ApiError> {
        Ok(Box::new(MemTarget {
            files: self.clone(),
            path: path.into(),
            data: Vec::new(),
        }))
    }
}

fn apply(s: &Session, doc: DocId, commands: Vec<EditCommand>) -> ApplyResult {
    match ok(s.dispatch(Request::Apply {
        doc,
        label: "test".into(),
        commands,
    }))
    .0
    {
        ResponseBody::Applied(a) => a,
        b => panic!("{b:?}"),
    }
}

fn summary(s: &Session, doc: DocId) -> DocSummary {
    match ok(s.dispatch(Request::Summary { doc })).0 {
        ResponseBody::Doc(d) => d,
        b => panic!("{b:?}"),
    }
}

#[test]
fn edit_undo_redo_and_save() {
    let exec = Arc::new(QueueExecutor::default());
    let files = MemFiles::default();
    let s = Session::new("test", files.clone(), exec.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    {
        let events = events.clone();
        s.subscribe(move |e| events.lock().unwrap().push(e.clone()));
    }
    let doc = match ok(s.dispatch(Request::Open {
        source: OpenSource::Path {
            path: "model.pdf".into(),
        },
        password: None,
        recover: false,
    }))
    .0
    {
        ResponseBody::Doc(d) => d.info.doc,
        b => panic!("{b:?}"),
    };
    assert!(!summary(&s, doc).edit.dirty);
    let r = apply(
        &s,
        doc,
        vec![
            EditCommand::DeletePages { pages: vec![0] },
            EditCommand::SetEntry {
                node: NodePath {
                    root: NodeRoot::Object {
                        id: ObjRef {
                            num: 1,
                            generation: 0,
                        },
                    },
                    path: vec![],
                },
                key: "PageMode".into(),
                // Not a page mode: the schema says so.
                value: "/Sideways".into(),
            },
        ],
    );
    assert!(r.status.dirty);
    assert_eq!(r.status.undo.as_deref(), Some("test"));
    assert!(r.changed.contains(&1));
    assert!(
        r.issues.iter().any(|i| i.message.contains("Sideways")),
        "{:?}",
        r.issues
    );
    assert!(events.lock().unwrap().iter().any(|e| matches!(
        e,
        Event::DocChanged {
            reason: ChangeReason::Edit,
            ..
        }
    )));
    let sum = summary(&s, doc);
    assert_eq!(sum.pages, 4);
    let o = match ok(s.dispatch(Request::Object {
        doc,
        id: ObjRef {
            num: 1,
            generation: 0,
        },
    }))
    .0
    {
        ResponseBody::Object(o) => o,
        b => panic!("{b:?}"),
    };
    assert_eq!(o.location, Location::Edited);
    assert_eq!(o.span, None);

    // A failing command leaves everything as it was.
    let r = s.dispatch(Request::Apply {
        doc,
        label: "bad".into(),
        commands: vec![
            EditCommand::DeletePages { pages: vec![0] },
            EditCommand::DeletePages { pages: vec![99] },
        ],
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::BadRequest));
    assert_eq!(summary(&s, doc).pages, 4);

    let st = match ok(s.dispatch(Request::Undo { doc })).0 {
        ResponseBody::Edit(st) => st,
        b => panic!("{b:?}"),
    };
    assert!(!st.dirty);
    assert_eq!(st.redo.as_deref(), Some("test"));
    assert_eq!(summary(&s, doc).pages, 5);
    ok(s.dispatch(Request::Redo { doc }));
    assert_eq!(summary(&s, doc).pages, 4);

    // Save as an update over the file, then it is reopened from there.
    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::Save {
            path: None,
            mode: SaveMode::Incremental,
        },
    }));
    let ResponseBody::Job(j) = b else { panic!() };
    exec.run_all();
    let info = match ok(s.dispatch(Request::JobStatus { job: j.job })).0 {
        ResponseBody::Job(i) => i,
        b => panic!("{b:?}"),
    };
    assert_eq!(info.state, JobState::Completed, "{:?}", info.error);
    let Some(JobResult::Save(r)) = info.result else {
        panic!("{info:?}")
    };
    assert_eq!(r.path.as_deref(), Some("model.pdf"));
    let saved = files.0.lock().unwrap()["model.pdf"].clone();
    assert!(saved.starts_with(&fixture("model.pdf")));
    assert_eq!(r.bytes, saved.len() as u64);
    let sum = summary(&s, doc);
    assert_eq!(sum.revisions.len(), 2);
    assert_eq!(sum.pages, 4);
    assert!(!sum.edit.dirty);
    assert!(sum.edit.undo.is_none());
    assert_eq!(sum.info.size, saved.len() as u64);
    assert!(events.lock().unwrap().iter().any(|e| matches!(
        e,
        Event::DocChanged {
            reason: ChangeReason::Saved,
            info: Some(_),
            ..
        }
    )));

    // A full rewrite to another file.
    let (b, _) = ok(s.dispatch(Request::StartJob {
        doc,
        job: JobKind::Save {
            path: Some("copy.pdf".into()),
            mode: SaveMode::Full {
                xref_stream: true,
                object_streams: true,
                compress: true,
                remove_unused: true,
            },
        },
    }));
    let ResponseBody::Job(j) = b else { panic!() };
    exec.run_all();
    let info = match ok(s.dispatch(Request::JobStatus { job: j.job })).0 {
        ResponseBody::Job(i) => i,
        b => panic!("{b:?}"),
    };
    assert_eq!(info.state, JobState::Completed, "{:?}", info.error);
    let sum = summary(&s, doc);
    assert_eq!(sum.info.path.as_deref(), Some("copy.pdf"));
    assert_eq!(sum.revisions.len(), 1);
    assert_eq!(sum.pages, 4);
}

#[test]
fn browser_save_stream() {
    let (s, _) = session();
    let doc = open(&s, "aes-256.pdf", Some("user")).info.doc;
    let st = summary(&s, doc).edit;
    assert!(st.no_incremental.is_some());
    assert!(s.save_stream(doc, SaveMode::Incremental).is_err());
    let mut w = s
        .save_stream(
            doc,
            SaveMode::Full {
                xref_stream: false,
                object_streams: false,
                compress: false,
                remove_unused: false,
            },
        )
        .unwrap();
    let mut out = Vec::new();
    while let Some(c) = w.next_chunk().unwrap() {
        out.extend(c);
    }
    assert_eq!(w.result().bytes, out.len() as u64);
    let sum = s
        .reload(doc, Arc::new(MemSource::new(out)), "saved.pdf")
        .unwrap();
    assert!(sum.encryption.is_none());
    assert_eq!(sum.info.doc, doc);
    assert_eq!(sum.pages, 3);
}

#[test]
fn insert_pages_from_another_document() {
    let (s, _) = session();
    let a = open(&s, "model.pdf", None).info.doc;
    let b = open(&s, "content.pdf", None).info.doc;
    let r = apply(
        &s,
        a,
        vec![EditCommand::InsertPages {
            at: 0,
            from: b,
            pages: vec![0, 1],
        }],
    );
    assert!(r.created.len() > 2);
    assert_eq!(summary(&s, a).pages, 7);
    let r = s.dispatch(Request::Apply {
        doc: a,
        label: "x".into(),
        commands: vec![EditCommand::InsertPages {
            at: 0,
            from: 99,
            pages: vec![0],
        }],
    });
    assert!(matches!(r.response, Response::Err(e) if e.code == ErrorCode::NoSuchDoc));
}

#[test]
fn node_syntax_round_trips() {
    let (s, _) = session();
    let doc = open(&s, "model.pdf", None).info.doc;
    let path = NodePath {
        root: NodeRoot::Object {
            id: ObjRef {
                num: 2,
                generation: 0,
            },
        },
        path: vec![],
    };
    let text = match ok(s.dispatch(Request::NodeSyntax {
        doc,
        path: path.clone(),
    }))
    .0
    {
        ResponseBody::Syntax { text, truncated } => {
            assert!(!truncated);
            text
        }
        b => panic!("{b:?}"),
    };
    assert!(text.starts_with("<<\n  /Type /Pages"), "{text}");
    // Written back unchanged, the pages stay the same.
    apply(
        &s,
        doc,
        vec![EditCommand::SetValue {
            node: path,
            value: text,
        }],
    );
    assert_eq!(summary(&s, doc).pages, 5);
}

fn content_summary(s: &Session, doc: DocId, content: ContentRef) -> ContentSummary {
    match ok(s.dispatch(Request::Content { doc, content })).0 {
        ResponseBody::Content(c) => c,
        b => panic!("{b:?}"),
    }
}

fn op_views(s: &Session, doc: DocId, content: ContentRef) -> Vec<OpView> {
    match ok(s.dispatch(Request::ContentOps {
        doc,
        content,
        range: all(),
    }))
    .0
    {
        ResponseBody::Ops(p) => p.items,
        b => panic!("{b:?}"),
    }
}

#[test]
fn edit_operations_of_a_content() {
    let (s, _) = session();
    let doc = open(&s, "content.pdf", None).info.doc;
    let page = ContentRef::Page { index: 0 };
    let n = content_summary(&s, doc, page).ops as u32;
    let text = match ok(s.dispatch(Request::ContentText {
        doc,
        content: page,
        range: all(),
    }))
    .0
    {
        ResponseBody::OpText(t) => {
            assert_eq!((t.from, t.to), (0, n));
            t.text
        }
        b => panic!("{b:?}"),
    };
    // The same text changes nothing.
    let same = s.dispatch(Request::Apply {
        doc,
        label: "same".into(),
        commands: vec![EditCommand::EditOps {
            content: page,
            edits: vec![OpEdit::Replace {
                from: 0,
                to: n,
                text: text.clone(),
            }],
        }],
    });
    match same.response {
        Response::Err(e) => assert!(e.message.contains("as it was"), "{e}"),
        r => panic!("{r:?}"),
    }

    // Wrap everything in q … Q.
    let r = apply(
        &s,
        doc,
        vec![EditCommand::EditOps {
            content: page,
            edits: vec![
                OpEdit::Replace {
                    from: 0,
                    to: 0,
                    text: "q".into(),
                },
                OpEdit::Replace {
                    from: n + 1,
                    to: n + 1,
                    text: "Q".into(),
                },
            ],
        }],
    );
    assert!(!r.changed.is_empty());
    let ops = op_views(&s, doc, page);
    assert_eq!(ops.len() as u32, n + 2);
    assert_eq!((ops[0].operator.as_str(), ops[0].end), ("q", Some(n + 1)));
    assert_eq!(ops[n as usize + 1].operator, "Q");
    let blocks = match ok(s.dispatch(Request::ContentBlocks {
        doc,
        content: page,
        depth: 0,
    }))
    .0
    {
        ResponseBody::Blocks { blocks, .. } => blocks,
        b => panic!("{b:?}"),
    };
    assert_eq!(blocks, [BlockSpan::from([0, n + 1])]);

    // Move the closing Q to the front, then undo.
    apply(
        &s,
        doc,
        vec![EditCommand::EditOps {
            content: page,
            edits: vec![OpEdit::Move {
                from: n + 1,
                to: n + 2,
                before: 0,
            }],
        }],
    );
    assert_eq!(op_views(&s, doc, page)[0].operator, "Q");
    ok(s.dispatch(Request::Undo { doc }));
    assert_eq!(op_views(&s, doc, page)[0].operator, "q");

    // Text that is not a byte, or not content syntax, is refused.
    for bad in ["(日本)Tj", "0 0 m (x"] {
        let r = s.dispatch(Request::Apply {
            doc,
            label: "bad".into(),
            commands: vec![EditCommand::EditOps {
                content: page,
                edits: vec![OpEdit::Replace {
                    from: 0,
                    to: 0,
                    text: bad.into(),
                }],
            }],
        });
        assert!(matches!(r.response, Response::Err(_)), "{bad}");
    }

    // A blank page gets a content stream.
    apply(
        &s,
        doc,
        vec![
            EditCommand::InsertBlankPage {
                at: 0,
                width: 100.0,
                height: 100.0,
            },
            EditCommand::EditOps {
                content: ContentRef::Page { index: 0 },
                edits: vec![OpEdit::Replace {
                    from: 0,
                    to: 0,
                    text: "1 0 0 rg 0 0 50 50 re f".into(),
                }],
            },
        ],
    );
    let blank = content_summary(&s, doc, ContentRef::Page { index: 0 });
    assert_eq!((blank.ops, blank.errors), (3, 0));
}
