use std::sync::Arc;

use pdfi_schema::{DocSchema, IssueCode, Origin, Walk, path_text};
use pdfi_syntax::{DictExt, MemSource, ObjId, OpenOptions, PdfFile};

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
fn typing_walk_types_without_issues() {
    // schema.pdf is full of problems: none is collected.
    let f = open("schema.pdf");
    let s = DocSchema::new();
    let ck = s.checker(&f);
    let mut issues = Vec::new();
    let mut w = Walk::types(&ck);
    while w.step(&ck, 4, &mut issues) {}
    assert!(issues.is_empty(), "{issues:?}");
    // The page is reached through /Kids: typed from context, not guessed.
    let (t, o) = ck.type_of(ObjId::new(3, 0)).unwrap();
    assert_eq!((t.name().as_str(), o), ("PageObject", Origin::Context));
}

#[test]
fn a_type_named_by_type_alone_is_not_a_guess() {
    let with_type = |f: &PdfFile, name: &str| {
        f.xref()
            .live_ids()
            .find(|&id| {
                f.get(id).is_ok_and(|o| {
                    o.node
                        .obj
                        .as_dict()
                        .and_then(|d| d.get_name("Type"))
                        .is_some_and(|n| n.as_bytes() == name.as_bytes())
                })
            })
            .unwrap()
    };
    // Nothing refers to an object stream, and only one type takes /ObjStm.
    let f = open("objstm.pdf");
    let s = DocSchema::new();
    let ck = s.checker(&f);
    let (t, o) = ck.type_of(with_type(&f, "ObjStm")).unwrap();
    assert_eq!(o, Origin::TypeKey, "{}", t.name());
    // /Type /Page is also taken by StructureAttributesDict: still a guess
    // until reached through /Kids.
    let f = open("base.pdf");
    let s = DocSchema::new();
    let ck = s.checker(&f);
    let (t, o) = ck.type_of(ObjId::new(3, 0)).unwrap();
    assert_eq!((t.name().as_str(), o), ("PageObject", Origin::Guess));
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
