use std::path::PathBuf;
use std::sync::Arc;

use pdfi_syntax::crypt::AuthResult;
use pdfi_syntax::write::{RewriteOptions, rewrite};
use pdfi_syntax::xref::XrefKind;
use pdfi_syntax::{DiagCode, DictExt, MemSource, Object, OpenOptions, PdfFile};

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

fn open(name: &str, password: Option<&str>) -> PdfFile {
    open_bytes(fixture(name), password)
}

/// Page dictionaries in document order.
fn pages(f: &PdfFile) -> Vec<pdfi_syntax::Dict> {
    let root = f.resolve(f.trailer().get_key("Root").unwrap()).unwrap();
    let mut out = Vec::new();
    let mut stack = vec![root.as_dict().unwrap().get_key("Pages").unwrap().clone()];
    while let Some(o) = stack.pop() {
        let o = f.resolve(&o).unwrap();
        let d = o.as_dict().unwrap().clone();
        if d.type_is("Pages") {
            let kids = f.resolve(d.get_key("Kids").unwrap()).unwrap();
            stack.extend(kids.as_array().unwrap().iter().rev().map(|k| k.obj.clone()));
        } else {
            out.push(d);
        }
    }
    out
}

fn title(f: &PdfFile) -> Vec<u8> {
    let info = f.resolve(f.trailer().get_key("Info").unwrap()).unwrap();
    info.as_dict()
        .unwrap()
        .get_key("Title")
        .unwrap()
        .as_string()
        .unwrap()
        .bytes
        .clone()
}

fn page_text(f: &PdfFile, page: &pdfi_syntax::Dict) -> String {
    let id = page.get_key("Contents").unwrap().as_ref_id().unwrap();
    let o = f.get(id).unwrap();
    let Object::Stream(s) = &o.node.obj else {
        panic!("contents is not a stream")
    };
    let d = f.decode_stream(id, s, None);
    assert!(d.errors.is_empty(), "{:?}", d.errors);
    String::from_utf8(d.data).unwrap()
}

fn assert_document(f: &PdfFile, expect_title: &[u8]) {
    let p = pages(f);
    assert_eq!(p.len(), 3);
    for (i, page) in p.iter().enumerate() {
        assert!(page_text(f, page).contains(&format!("(Page {}) Tj", i + 1)));
    }
    assert_eq!(title(f), expect_title);
}

#[test]
fn plain_and_variants() {
    for name in ["base.pdf", "objstm.pdf", "linearized.pdf"] {
        let f = open(name, None);
        assert!(!f.is_recovered(), "{name}");
        assert_document(&f, b"Fixture (test)");
    }
    let f = open("objstm.pdf", None);
    assert_eq!(f.revisions()[0].kind, XrefKind::Stream);
}

#[test]
fn incremental_update() {
    let f = open("incremental.pdf", None);
    assert_eq!(f.revisions().len(), 2);
    assert_document(&f, b"Updated");
    assert_eq!(
        f.get_num(14).unwrap().node.obj.as_string().unwrap().bytes,
        b"new object"
    );
}

#[test]
fn encrypted_with_passwords() {
    for name in [
        "rc4-40.pdf",
        "rc4-128.pdf",
        "aes-128.pdf",
        "aes-256.pdf",
        "aes-256-objstm.pdf",
    ] {
        let f = open(name, Some("user"));
        assert_eq!(f.security().unwrap().auth, Some(AuthResult::User), "{name}");
        assert_document(&f, b"Fixture (test)");
        let f = open(name, Some("owner"));
        assert_eq!(
            f.security().unwrap().auth,
            Some(AuthResult::Owner),
            "{name}"
        );
        assert_document(&f, b"Fixture (test)");
        let f = open(name, None);
        assert!(!f.security().unwrap().is_authenticated(), "{name}");
        assert!(
            f.diagnostics()
                .iter()
                .any(|d| d.code == DiagCode::EncryptionError)
        );
    }
    let f = open("aes-256-nopw.pdf", None);
    assert_eq!(f.security().unwrap().auth, Some(AuthResult::User));
    assert_document(&f, b"Fixture (test)");
}

#[test]
fn broken_file_is_recovered() {
    let f = open("broken.pdf", None);
    assert_document(&f, b"Fixture (test)");
    let codes: Vec<_> = f.diagnostics().iter().map(|d| d.code).collect();
    assert!(codes.contains(&DiagCode::StreamLengthInvalid), "{codes:?}");
}

#[test]
fn rewrite_roundtrip() {
    for (name, pw) in [
        ("base.pdf", None),
        ("objstm.pdf", None),
        ("incremental.pdf", None),
        ("aes-256-objstm.pdf", Some("user")),
        ("rc4-128.pdf", Some("owner")),
        ("broken.pdf", None),
    ] {
        let f = open(name, pw);
        for copy_unchanged in [true, false] {
            let mut out = Vec::new();
            let rep = rewrite(&f, &mut out, &RewriteOptions { copy_unchanged }).unwrap();
            assert!(rep.failed.is_empty(), "{name}: {:?}", rep.failed);
            let g = open_bytes(out, None);
            assert!(g.security().is_none());
            assert!(!g.is_recovered(), "{name}: {:?}", g.diagnostics());
            let expect = if name == "incremental.pdf" {
                b"Updated".as_slice()
            } else {
                b"Fixture (test)"
            };
            assert_document(&g, expect);
        }
    }
}
