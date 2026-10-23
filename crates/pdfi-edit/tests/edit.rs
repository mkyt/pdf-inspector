use std::path::PathBuf;
use std::process::Command as Process;
use std::sync::Arc;

use pdfi_edit::{
    Command, FullOptions, History, NodeAt, Root, SaveMode, Saver, StreamFilter, Tx, parse_value,
};
use pdfi_model::DocModel;
use pdfi_model::xmp::{DC, PDF};
use pdfi_syntax::xref::XrefKind;
use pdfi_syntax::{DictExt, MemSource, Name, ObjId, Object, OpenOptions, PdfFile};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn open_bytes(data: Vec<u8>, password: Option<&str>) -> PdfFile {
    let opts = OpenOptions {
        password: password.map(|p| p.as_bytes().to_vec()),
        ..Default::default()
    };
    PdfFile::open(Arc::new(MemSource::new(data)), opts).unwrap()
}

fn open(name: &str) -> PdfFile {
    open_bytes(fixture(name), None)
}

fn edit(f: &PdfFile, cmds: Vec<Command>) -> PdfFile {
    let (edits, _) = Tx::apply_all(f, cmds).unwrap();
    f.with_edits(edits)
}

fn save(f: &PdfFile, mode: SaveMode) -> Vec<u8> {
    let mut out = Vec::new();
    Saver::new(f, mode).unwrap().write_all(&mut out).unwrap();
    out
}

/// `qpdf --check` agrees the output is sound (skipped without qpdf).
fn qpdf_check(data: &[u8], password: Option<&str>) {
    let dir = std::env::temp_dir().join(format!("pdfi-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{:x}.pdf", md5_len(data)));
    std::fs::write(&p, data).unwrap();
    let mut c = Process::new("qpdf");
    c.arg("--check");
    if let Some(pw) = password {
        c.arg(format!("--password={pw}"));
    }
    let Ok(out) = c.arg(&p).output() else {
        return;
    };
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && !text.contains("WARNING"),
        "qpdf --check {}:\n{text}",
        p.display()
    );
    let _ = std::fs::remove_file(p);
}

fn md5_len(d: &[u8]) -> u64 {
    d.iter().fold(d.len() as u64, |h, &b| {
        h.wrapping_mul(31).wrapping_add(b as u64)
    })
}

fn page_ids(f: &PdfFile) -> Vec<u32> {
    let dm = DocModel::new();
    let m = dm.on(f);
    (0..m.page_count())
        .map(|i| m.page(i).unwrap().id.num)
        .collect()
}

fn obj(f: &PdfFile, num: u32) -> Object {
    f.get_num(num).unwrap().node.obj.clone()
}

fn at(num: u32, path: &[usize]) -> NodeAt {
    NodeAt {
        root: Root::Object(ObjId::new(num, 0)),
        path: path.to_vec(),
    }
}

#[test]
fn incremental_update_appends() {
    let src = fixture("base.pdf");
    let f = open_bytes(src.clone(), None);
    let root = f.trailer().get_key("Root").unwrap().as_ref_id().unwrap();
    let e = edit(
        &f,
        vec![
            Command::SetEntry {
                at: NodeAt {
                    root: Root::Object(root),
                    path: vec![],
                },
                key: Name::from("PageMode"),
                value: parse_value("/UseThumbs").unwrap(),
            },
            Command::AddObject {
                value: parse_value("<< /Hello (world) >>").unwrap(),
                data: None,
            },
        ],
    );
    let out = save(&e, SaveMode::Incremental);
    assert!(out.starts_with(&src), "the file's bytes come first");
    let g = open_bytes(out.clone(), None);
    assert_eq!(g.revisions().len(), f.revisions().len() + 1);
    let cat = g.resolve(g.trailer().get_key("Root").unwrap()).unwrap();
    assert_eq!(
        cat.as_dict().unwrap().get_name("PageMode").unwrap(),
        "UseThumbs"
    );
    let added = ObjId::new(f.xref().size(), 0);
    assert!(g.get(added).is_ok());
    assert!(g.diagnostics().is_empty(), "{:?}", g.diagnostics());
    qpdf_check(&out, None);
}

#[test]
fn incremental_after_xref_stream() {
    let f = open("objstm.pdf");
    assert_eq!(f.revisions()[0].kind, XrefKind::Stream);
    let num = f
        .xref()
        .iter()
        .find(|(_, e)| matches!(e, pdfi_syntax::xref::XrefEntry::Compressed { .. }))
        .unwrap()
        .0;
    let o = f.get_num(num).unwrap();
    let e = edit(
        &f,
        vec![Command::SetEntry {
            at: NodeAt {
                root: Root::Object(o.id),
                path: vec![],
            },
            key: Name::from("Replaced"),
            value: parse_value("true").unwrap(),
        }],
    );
    let out = save(&e, SaveMode::Incremental);
    let g = open_bytes(out.clone(), None);
    assert_eq!(g.revisions()[0].kind, XrefKind::Stream);
    assert_eq!(
        obj(&g, num).as_dict().unwrap().get_key("Replaced"),
        Some(&Object::Bool(true))
    );
    assert!(g.diagnostics().is_empty(), "{:?}", g.diagnostics());
    qpdf_check(&out, None);
}

#[test]
fn deleted_objects_are_freed() {
    let f = open("model.pdf");
    let e = edit(
        &f,
        vec![Command::DeleteObject {
            id: ObjId::new(90, 0),
        }],
    );
    assert!(e.get(ObjId::new(90, 0)).is_err());
    let g = open_bytes(save(&e, SaveMode::Incremental), None);
    assert!(g.get(ObjId::new(90, 0)).is_err());
    assert!(!g.xref().get(90).unwrap().is_live());
    assert_eq!(g.xref().get(90).unwrap().generation(), 1);
}

#[test]
fn page_operations() {
    let f = open("model.pdf");
    assert_eq!(page_ids(&f), [10, 11, 12, 13, 14]);
    let e = edit(&f, vec![Command::DeletePages { pages: vec![1] }]);
    assert_eq!(page_ids(&e), [10, 12, 13, 14]);
    // Moving the last page to the front: under another parent, it keeps
    // what it inherited.
    let e = edit(
        &f,
        vec![Command::MovePages {
            pages: vec![3],
            to: 0,
        }],
    );
    assert_eq!(page_ids(&e), [13, 10, 11, 12, 14]);
    let p13 = obj(&e, 13);
    assert!(p13.as_dict().unwrap().get_key("MediaBox").is_some());
    let e = edit(
        &f,
        vec![Command::MovePages {
            pages: vec![0, 1],
            to: 3,
        }],
    );
    assert_eq!(page_ids(&e), [12, 13, 14, 10, 11]);
    let e = edit(
        &f,
        vec![Command::RotatePages {
            pages: vec![0, 2],
            by: 90,
        }],
    );
    assert_eq!(obj(&e, 10).as_dict().unwrap().get_i64("Rotate"), Some(180));
    assert_eq!(obj(&e, 12).as_dict().unwrap().get_i64("Rotate"), Some(90));
    let e = edit(
        &f,
        vec![
            Command::InsertBlankPage {
                at: 2,
                width: 595.0,
                height: 842.0,
            },
            Command::InsertBlankPage {
                at: 99,
                width: 100.0,
                height: 100.5,
            },
        ],
    );
    let ids = page_ids(&e);
    assert_eq!(ids.len(), 7);
    assert_eq!(&ids[..2], [10, 11]);
    assert_eq!(&ids[3..6], [12, 13, 14]);
    // Inserted first, under the node that sets a crop box and a rotation:
    // the new page does not take them.
    let e = edit(
        &f,
        vec![Command::InsertBlankPage {
            at: 0,
            width: 200.0,
            height: 100.0,
        }],
    );
    let dm = DocModel::new();
    let info = dm.on(&e).page_info(0).unwrap();
    assert_eq!(info.rotate, 0);
    assert_eq!(info.visible().unwrap().width(), 200.0);
    // Deleting a whole subtree removes the empty node too.
    let e = edit(&f, vec![Command::DeletePages { pages: vec![0, 1] }]);
    assert_eq!(page_ids(&e), [12, 13, 14]);
    let kids = obj(&e, 2);
    let kids = kids
        .as_dict()
        .unwrap()
        .get_key("Kids")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(kids.len(), 2);
    let out = save(&e, SaveMode::Incremental);
    qpdf_check(&out, None);
    assert_eq!(page_ids(&open_bytes(out, None)), [12, 13, 14]);
}

#[test]
fn copy_pages_between_files() {
    let f = open("model.pdf");
    let other = open("content.pdf");
    let e = edit(
        &f,
        vec![Command::InsertPages {
            at: 1,
            from: Some(&other),
            pages: vec![0, 2],
        }],
    );
    let ids = page_ids(&e);
    assert_eq!(ids.len(), 7);
    assert_eq!(ids[0], 10);
    assert_eq!(&ids[3..], [11, 12, 13, 14]);
    for opts in [
        FullOptions::default(),
        FullOptions {
            xref_stream: true,
            object_streams: true,
            compress: true,
            remove_unused: true,
        },
    ] {
        let out = save(&e, SaveMode::Full(opts));
        qpdf_check(&out, None);
        let g = open_bytes(out, None);
        assert_eq!(page_ids(&g).len(), 7);
        assert!(g.diagnostics().is_empty(), "{:?}", g.diagnostics());
    }
    // Within the same file the resources are shared.
    let e = edit(
        &f,
        vec![Command::InsertPages {
            at: 5,
            from: None,
            pages: vec![4],
        }],
    );
    let ids = page_ids(&e);
    assert_eq!(ids.len(), 6);
    let copy = obj(&e, ids[5]);
    let res = copy.as_dict().unwrap().get_key("Resources").unwrap();
    let fonts = res.as_dict().unwrap().get_key("Font").unwrap();
    assert_eq!(
        fonts.as_dict().unwrap().get_key("F1").unwrap().as_ref_id(),
        Some(ObjId::new(20, 0))
    );
}

#[test]
fn info_and_xmp() {
    let f = open("model.pdf");
    let e = edit(
        &f,
        vec![
            Command::SetInfo {
                key: "Title".into(),
                value: Some("新しい題".into()),
                sync_xmp: true,
            },
            Command::SetInfo {
                key: "ModDate".into(),
                value: Some("2026-10-24T09:30:00+09:00".into()),
                sync_xmp: true,
            },
            Command::SetInfo {
                key: "Producer".into(),
                value: None,
                sync_xmp: true,
            },
        ],
    );
    let dm = DocModel::new();
    let m = dm.on(&e);
    let (_, entries, _) = m.info_entries();
    let get = |k: &str| entries.iter().find(|x| x.key == k).map(|x| x.text.clone());
    assert_eq!(get("Title").as_deref(), Some("新しい題"));
    assert_eq!(get("ModDate").as_deref(), Some("D:20261024093000+09'00'"));
    assert_eq!(get("Producer"), None);
    let meta = m.metadata().unwrap();
    assert_eq!(meta.xmp.property(DC, "title"), Some("新しい題"));
    assert_eq!(meta.xmp.property(PDF, "Producer"), None);
    assert_eq!(
        meta.xmp.property(pdfi_model::xmp::XMP, "ModifyDate"),
        Some("2026-10-24T09:30:00+09:00")
    );
    qpdf_check(&save(&e, SaveMode::Incremental), None);
}

#[test]
fn stream_data_and_filters() {
    let f = open("model.pdf");
    let id = ObjId::new(15, 0);
    let e = edit(
        &f,
        vec![Command::SetStreamData {
            id,
            data: b"BT /F1 20 Tf 72 700 Td (Edited) Tj ET\n".to_vec(),
            filter: StreamFilter::Flate,
        }],
    );
    let o = e.get(id).unwrap();
    let s = o.node.obj.as_stream().unwrap();
    assert_eq!(s.dict.get_name("Filter").unwrap(), "FlateDecode");
    assert_eq!(
        e.decode_stream(id, s, None).data,
        b"BT /F1 20 Tf 72 700 Td (Edited) Tj ET\n"
    );
    let e2 = edit(
        &e,
        vec![Command::SetStreamFilter {
            id,
            filter: StreamFilter::None,
        }],
    );
    let o = e2.get(id).unwrap();
    let s = o.node.obj.as_stream().unwrap();
    assert!(s.dict.get_key("Filter").is_none());
    let out = save(&e2, SaveMode::Incremental);
    qpdf_check(&out, None);
    let g = open_bytes(out, None);
    let o = g.get(id).unwrap();
    assert_eq!(
        g.decode_stream(id, o.node.obj.as_stream().unwrap(), None)
            .data,
        b"BT /F1 20 Tf 72 700 Td (Edited) Tj ET\n"
    );
}

#[test]
fn edits_at_paths() {
    let f = open("model.pdf");
    // /Kids of the root page tree node is entry 1.
    let e = edit(
        &f,
        vec![
            Command::Insert {
                at: at(2, &[1]),
                index: 3,
                value: Object::Null,
            },
            Command::RemoveChild {
                at: at(2, &[1]),
                index: 3,
            },
            Command::SetValue {
                at: at(80, &[0]),
                value: parse_value("(Retitled)").unwrap(),
            },
            Command::RemoveEntry {
                at: at(80, &[]),
                key: Name::from("Trapped"),
            },
        ],
    );
    let info = obj(&e, 80);
    let info = info.as_dict().unwrap();
    assert_eq!(
        info.get_key("Title").unwrap().as_string().unwrap().bytes,
        b"Retitled"
    );
    assert!(info.get_key("Trapped").is_none());
    assert_eq!(page_ids(&e), page_ids(&f));
    let bad = Tx::apply_all(
        &f,
        vec![Command::Insert {
            at: at(80, &[]),
            index: 0,
            value: Object::Null,
        }],
    );
    assert!(bad.is_err());
}

#[test]
fn encrypted_files() {
    let f = open_bytes(fixture("aes-256.pdf"), Some("user"));
    let e = edit(
        &f,
        vec![Command::SetInfo {
            key: "Title".into(),
            value: Some("Plain now".into()),
            sync_xmp: false,
        }],
    );
    assert!(Saver::new(&e, SaveMode::Incremental).is_err());
    let out = save(&e, SaveMode::Full(FullOptions::default()));
    qpdf_check(&out, None);
    let g = open_bytes(out, None);
    assert!(g.security().is_none());
    assert_eq!(page_ids(&g).len(), 3);
    let dm = DocModel::new();
    let (_, entries, _) = dm.on(&g).info_entries();
    assert!(
        entries
            .iter()
            .any(|x| x.key == "Title" && x.text == "Plain now")
    );
}

#[test]
fn full_rewrite_drops_unused() {
    let f = open("model.pdf");
    let opts = FullOptions {
        remove_unused: true,
        ..Default::default()
    };
    let out = save(&f, SaveMode::Full(opts));
    qpdf_check(&out, None);
    let g = open_bytes(out, None);
    assert!(g.get(ObjId::new(90, 0)).is_err(), "the orphan is gone");
    assert_eq!(page_ids(&g), [10, 11, 12, 13, 14]);
    let lin = open("linearized.pdf");
    let g = open_bytes(save(&lin, SaveMode::Full(FullOptions::default())), None);
    assert!(g.xref().live_ids().all(|id| {
        g.get(id)
            .unwrap()
            .node
            .obj
            .as_dict()
            .is_none_or(|d| !d.contains_key(b"Linearized".as_slice()))
    }));
}

#[test]
fn history() {
    let f = open("model.pdf");
    let mut h = History::default();
    let (e1, _) = Tx::apply_all(&f, vec![Command::DeletePages { pages: vec![0] }]).unwrap();
    h.push("delete", e1);
    let s1 = f.with_edits(h.current().clone());
    let (e2, _) = Tx::apply_all(&s1, vec![Command::DeletePages { pages: vec![0] }]).unwrap();
    h.push("delete again", e2);
    assert_eq!(page_ids(&f.with_edits(h.current().clone())), [12, 13, 14]);
    assert_eq!(h.undo_label(), Some("delete again"));
    h.undo();
    assert_eq!(
        page_ids(&f.with_edits(h.current().clone())),
        [11, 12, 13, 14]
    );
    h.undo();
    assert!(!h.dirty());
    assert!(h.undo().is_none());
    h.redo();
    assert_eq!(h.redo_label(), Some("delete again"));
    // A new edit drops what was undone.
    let s = f.with_edits(h.current().clone());
    let (e3, _) = Tx::apply_all(
        &s,
        vec![Command::RotatePages {
            pages: vec![0],
            by: 90,
        }],
    )
    .unwrap();
    h.push("rotate", e3);
    assert!(h.redo_label().is_none());
}
