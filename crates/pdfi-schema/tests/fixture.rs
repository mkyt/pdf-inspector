use std::sync::Arc;

use pdfi_schema::{DocSchema, IssueCode, Origin, Walk, path_text};
use pdfi_syntax::{MemSource, ObjId, OpenOptions, PdfFile};

fn open(name: &str) -> PdfFile {
    let p = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read(&p).unwrap();
    PdfFile::open(Arc::new(MemSource::new(data)), OpenOptions::default()).unwrap()
}

#[test]
fn clean_file_has_no_issues_and_is_typed() {
    let f = open("base.pdf");
    let s = DocSchema::new();
    let ck = s.checker(&f);
    let mut issues = Vec::new();
    let mut w = Walk::document(&ck);
    while w.step(&ck, 4, &mut issues) {}
    assert!(issues.is_empty(), "{issues:?}");
    let (t, o) = ck.type_of(ObjId::new(3, 0)).unwrap();
    assert_eq!((t.name().as_str(), o), ("PageObject", Origin::Context));
    assert_eq!(ck.type_of(ObjId::new(6, 0)).unwrap().0.name(), "FontType1");
}

#[test]
fn every_kind_of_problem_is_found() {
    let f = open("schema.pdf");
    let s = DocSchema::new();
    let ck = s.checker(&f);
    let mut issues = Vec::new();
    let mut w = Walk::document(&ck);
    while w.step(&ck, 64, &mut issues) {}
    let found: Vec<(u32, String, IssueCode)> = issues
        .iter()
        .map(|i| (i.obj.map_or(0, |o| o.num), path_text(&i.path), i.code))
        .collect();
    for want in [
        (1, "/PageLayout", IssueCode::BadValue),
        (1, "/Outlines", IssueCode::NotIndirect),
        (1, "/MarkInfo", IssueCode::TooNew),
        (1, "/Metadata", IssueCode::MissingObject),
        (2, "", IssueCode::MissingKey),
        (3, "/MediaBox", IssueCode::WrongType),
        (3, "/Rotate", IssueCode::PredicateFailed),
        (3, "/Contents", IssueCode::WrongType),
        (3, "/Foo", IssueCode::UnknownKey),
        (4, "/CreationDate", IssueCode::BadDate),
    ] {
        assert!(
            found.contains(&(want.0, want.1.to_string(), want.2)),
            "{want:?} not in {found:?}"
        );
    }
}
