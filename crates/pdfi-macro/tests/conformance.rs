//! The bundled macros on the native engine (QuickJS). This runner defines
//! `tests/conformance/macros-expected.json` (`UPDATE_CONFORMANCE=1`); the
//! browser's engine (`apps/web/scripts/conformance.ts`, through the wasm
//! session) must give the same results.

use std::path::PathBuf;
use std::sync::Arc;

use pdfi_api::*;
use pdfi_macro::{Library, MacroError, RunOptions};
use pdfi_syntax::MemSource;
use serde::Deserialize;
use serde_json::{Value, json};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

#[derive(Deserialize)]
struct Cases {
    cases: Vec<Case>,
    probes: Vec<Request>,
}

#[derive(Deserialize)]
struct Case {
    #[serde(rename = "macro")]
    name: String,
    file: String,
    params: serde_json::Map<String, Value>,
}

/// FNV-1a (32 bits), to compare emitted files without storing them.
fn fnv(data: &[u8]) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

fn run_case(c: &Case, probes: &[Request]) -> Value {
    let s = Arc::new(Session::new("direct", NoFilesystem, ThreadExecutor));
    let data = std::fs::read(conformance::fixtures().join(&c.file)).unwrap();
    let size = data.len();
    let doc = s
        .open_source(Arc::new(MemSource::new(data)), c.file.clone(), None, false)
        .unwrap()
        .info
        .doc;
    let (src, meta) = Library::new(vec![]).find(&c.name).unwrap();
    let opts = RunOptions {
        host: "test".into(),
        input: json!({ "name": c.file, "size": size }),
        ..RunOptions::default()
    };
    let r = pdfi_macro::run(&s, doc, &src.file, &src.code, &meta, &c.params, opts);
    let mut out = json!({ "macro": c.name, "file": c.file });
    match r {
        Ok(o) => {
            out["ok"] = json!(true);
            out["value"] = o.value;
            out["columns"] = json!(o.columns);
            out["emitted"] = o
                .emitted
                .iter()
                .map(|e| json!({ "name": e.name, "mime": e.mime, "size": e.data.len(), "fnv": fnv(&e.data) }))
                .collect();
        }
        Err(e) => {
            out["ok"] = json!(false);
            out["error"] = json!(match e {
                MacroError::Script { message, .. } => message,
                e => e.to_string(),
            });
        }
    }
    let edit = match s.dispatch(Request::Summary { doc }).response {
        Response::Ok(ResponseBody::Doc(d)) => d.edit,
        r => panic!("{r:?}"),
    };
    out["edit"] = json!(edit);
    out["probes"] = probes
        .iter()
        .map(|p| {
            let r = s.dispatch(p.clone());
            conformance::entry(&r.response, &r.blobs, &[])
        })
        .collect();
    out
}

#[test]
fn bundled_macros() {
    let text = std::fs::read_to_string(dir().join("macros.json")).unwrap();
    let cases: Cases = serde_json::from_str(&text).unwrap();
    let actual: Vec<Value> = cases
        .cases
        .iter()
        .map(|c| run_case(c, &cases.probes))
        .collect();
    let p = dir().join("macros-expected.json");
    if std::env::var_os("UPDATE_CONFORMANCE").is_some() {
        std::fs::write(&p, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(actual.len(), expected.len(), "number of cases");
    for (a, e) in actual.iter().zip(&expected) {
        assert!(
            a == e,
            "{} on {} differs from macros-expected.json\n  actual:   {a}\n  expected: {e}\n\
             (UPDATE_CONFORMANCE=1 cargo test -p pdfi-macro --test conformance regenerates it)",
            a["macro"],
            a["file"]
        );
    }
}
