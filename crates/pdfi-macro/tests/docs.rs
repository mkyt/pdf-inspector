//! The ```js macro examples of docs/macros run against a test document,
//! so that they stay right.

use std::path::PathBuf;
use std::sync::Arc;

use pdfi_api::{NoFilesystem, Session, ThreadExecutor};
use pdfi_macro::{RunOptions, read_meta};
use pdfi_syntax::MemSource;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `(file, line, code)` of every ```` ```js macro ```` block.
fn examples() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let dir = root().join("docs/macros");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let mut block: Option<(usize, String)> = None;
        for (i, line) in text.lines().enumerate() {
            match &mut block {
                None if line.trim() == "```js macro" => block = Some((i + 1, String::new())),
                Some((start, code)) if line.trim() == "```" => {
                    out.push((name.clone(), *start, std::mem::take(code)));
                    block = None;
                }
                Some((_, code)) => {
                    code.push_str(line);
                    code.push('\n');
                }
                None => {}
            }
        }
    }
    out
}

#[test]
fn doc_examples_run() {
    let fixture = std::fs::read(root().join("tests/fixtures/model.pdf")).unwrap();
    let all = examples();
    assert!(all.len() >= 8, "found {} examples", all.len());
    for (file, line, code) in all {
        let at = format!("{file}:{line}");
        let s = Arc::new(Session::new("test", NoFilesystem, ThreadExecutor));
        let doc = s
            .open_source(
                Arc::new(MemSource::new(fixture.clone())),
                "model.pdf",
                None,
                false,
            )
            .unwrap()
            .info
            .doc;
        let meta = read_meta(&at, &code, "example").unwrap_or_else(|e| panic!("{at}: {e}"));
        let out = pdfi_macro::run(
            &s,
            doc,
            &at,
            &code,
            &meta,
            &Default::default(),
            RunOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{at}: {e}"));
        assert!(!out.value.is_null(), "{at}: returned nothing");
    }
}
