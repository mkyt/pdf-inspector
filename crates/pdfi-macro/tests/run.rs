use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use pdfi_api::*;
use pdfi_macro::{Level, Library, MacroError, Meta, Outcome, RunOptions, read_meta};
use pdfi_syntax::MemSource;
use serde_json::{Map, Value, json};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn session(name: &str) -> (Arc<Session>, DocId) {
    let s = Arc::new(Session::new("test", NoFilesystem, ThreadExecutor));
    let d = s
        .open_source(Arc::new(MemSource::new(fixture(name))), name, None, false)
        .unwrap();
    (s, d.info.doc)
}

fn params(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn exec(
    s: &Arc<Session>,
    doc: DocId,
    code: &str,
    p: Value,
) -> (Result<Outcome, MacroError>, Vec<(Level, String)>) {
    let meta = read_meta("test.js", code, "test").unwrap();
    exec_meta(s, doc, code, &meta, p, RunOptions::default())
}

fn exec_meta(
    s: &Arc<Session>,
    doc: DocId,
    code: &str,
    meta: &Meta,
    p: Value,
    mut opts: RunOptions,
) -> (Result<Outcome, MacroError>, Vec<(Level, String)>) {
    let logs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let l = logs.clone();
    opts.log = Box::new(move |lv, t| l.lock().unwrap().push((lv, t.to_string())));
    let r = pdfi_macro::run(s, doc, "test.js", code, meta, &params(p), opts);
    let logs = logs.lock().unwrap().clone();
    (r, logs)
}

fn bundled(name: &str) -> (pdfi_macro::MacroSource, Meta) {
    Library::new(vec![]).resolve(name).unwrap()
}

fn run_bundled(s: &Arc<Session>, doc: DocId, name: &str, p: Value) -> Outcome {
    let (src, meta) = bundled(name);
    let (r, _) = exec_meta(s, doc, &src.code, &meta, p, RunOptions::default());
    r.unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn summary(s: &Session, doc: DocId) -> DocSummary {
    match s.dispatch(Request::Summary { doc }).response {
        Response::Ok(ResponseBody::Doc(d)) => d,
        r => panic!("{r:?}"),
    }
}

#[test]
fn bundled_macros_have_meta() {
    let names: Vec<String> = Library::new(vec![])
        .list()
        .into_iter()
        .map(|l| {
            let m = l.meta.unwrap_or_else(|e| panic!("{}: {e}", l.source.file));
            assert!(!m.description.is_empty());
            m.name
        })
        .collect();
    assert_eq!(
        names,
        [
            "fonts",
            "metadata",
            "set-reading",
            "crop-to-content",
            "remove-owner-password"
        ]
    );
    let (_, m) = bundled("set-reading");
    assert!(m.edits);
    assert_eq!(m.params[0].0, "layout");
    assert_eq!(m.param("direction").unwrap().default, Some(json!("R2L")));
}

#[test]
fn fonts() {
    let (s, doc) = session("fonts.pdf");
    let o = run_bundled(&s, doc, "fonts", json!({}));
    let rows = o.value.as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|r| r["font"].is_string() && r["pages"].is_number()),
        "{rows:?}"
    );
    assert!(!summary(&s, doc).edit.dirty);
}

#[test]
fn metadata() {
    let (s, doc) = session("model.pdf");
    let o = run_bundled(&s, doc, "metadata", json!({}));
    assert_eq!(o.value["pages"], 5);
    assert!(o.value["info"].is_object(), "{}", o.value);
}

#[test]
fn set_reading_is_one_undo_step() {
    let (s, doc) = session("model.pdf");
    let o = run_bundled(&s, doc, "set-reading", json!({ "layout": "TwoPageLeft" }));
    assert_eq!(
        o.value,
        json!({ "layout": "TwoPageLeft", "direction": "R2L" })
    );
    let m = run_bundled(&s, doc, "metadata", json!({}));
    assert_eq!(m.value["catalog"]["PageLayout"], "TwoPageLeft");
    assert_eq!(m.value["catalog"]["ViewerPreferences"]["Direction"], "R2L");
    let st = summary(&s, doc).edit;
    assert_eq!(
        st.undo.as_deref(),
        Some("Set page layout and reading direction")
    );
    // Both edits go with one undo.
    assert!(matches!(
        s.dispatch(Request::Undo { doc }).response,
        Response::Ok(_)
    ));
    assert!(!summary(&s, doc).edit.dirty);
}

#[test]
fn crop_to_content() {
    let (s, doc) = session("content.pdf");
    let o = run_bundled(
        &s,
        doc,
        "crop-to-content",
        json!({ "margin": "2", "pages": "1" }),
    );
    let rows = o.value.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{}", o.value);
    assert_eq!(rows[0]["page"], 1);
    let detail = match s.dispatch(Request::Page { doc, index: 0 }).response {
        Response::Ok(ResponseBody::PageDetail(p)) => p,
        r => panic!("{r:?}"),
    };
    let crop = detail.boxes.iter().find(|b| b.name == "CropBox").unwrap();
    assert_eq!(crop.origin, AttrOrigin::Page);
    let r = crop.rect.unwrap();
    assert!((r.x0 - rows[0]["x0"].as_f64().unwrap()).abs() < 0.01);
}

#[test]
fn remove_owner_password() {
    let (s, doc) = session("aes-256-nopw.pdf");
    let o = run_bundled(&s, doc, "remove-owner-password", json!({}));
    assert_eq!(o.value["encrypted"], true);
    let (_, meta) = bundled("remove-owner-password");
    assert!(matches!(meta.save.mode(), SaveMode::Full { .. }));
}

#[test]
fn failure_reverts_the_edits() {
    let (s, doc) = session("model.pdf");
    let code = r#"
        export const meta = { api: 1, title: "Failing", edits: true };
        export default async function (doc) {
            await doc.deletePages([0]);
            if (doc.pageCount !== 4) throw new Error("not deleted");
            await doc.setInfo({ Title: "changed" });
            throw new Error("on purpose");
        }"#;
    let (r, _) = exec(&s, doc, code, json!({}));
    match r {
        Err(MacroError::Script { message, stack, .. }) => {
            assert_eq!(message, "on purpose");
            assert!(stack.unwrap().contains("test.js"));
        }
        r => panic!("{r:?}"),
    }
    let sum = summary(&s, doc);
    assert_eq!(sum.pages, 5);
    assert!(!sum.edit.dirty);
}

#[test]
fn values_and_nodes() {
    let (s, doc) = session("model.pdf");
    let code = r#"
        export const meta = { api: 1, edits: true };
        export default async function (doc, _p, ctx) {
            const c = doc.catalog;
            const type = await c.get("/Type");
            const pages = await c.get("Pages");
            const page = await doc.page(1);
            const box = (await page.boxes()).media;
            await c.set("Custom", { A: [1, 2.5, true, null, pdf.name("N a")], B: "text", C: "日本", D: pdf.str("lit"), E: pdf.raw("<< /Z 1 >>") });
            const custom = await c.dict("Custom");
            const a = await custom.get("A");
            const e = await custom.dict("E");
            const removed = await custom.remove("B");
            ctx.log("logged", { x: 1 });
            console.warn("warned");
            const text = new TextDecoder().decode(new TextEncoder().encode("é"));
            return {
                type: type instanceof pdf.Name ? type.name : null,
                pagesIsRef: pages instanceof pdf.Ref,
                pageRef: page.ref.num > 0,
                box,
                a,
                c: (await custom.get("C")).text,
                d: (await custom.get("D")).text,
                z: await e.get("Z"),
                removed,
                keys: await custom.keys(),
                text,
                rect: page.size !== null,
            };
        }"#;
    let (r, logs) = exec(&s, doc, code, json!({}));
    let v = r.unwrap().value;
    assert_eq!(v["type"], "Catalog");
    assert_eq!(v["pagesIsRef"], true);
    assert_eq!(v["a"], json!([1, 2.5, true, null, "N a"]));
    assert_eq!(v["c"], "日本");
    assert_eq!(v["d"], "lit");
    assert_eq!(v["z"], 1);
    assert_eq!(v["removed"], true);
    assert_eq!(v["keys"], json!(["A", "C", "D", "E"]));
    assert_eq!(v["text"], "é");
    assert!(v["box"]["x1"].as_f64().unwrap() > 0.0);
    assert_eq!(
        logs,
        [
            (Level::Info, "logged {\"x\":1}".to_string()),
            (Level::Warn, "warned".to_string())
        ]
    );
}

#[test]
fn params_are_checked() {
    let (s, doc) = session("model.pdf");
    let code = r#"
        export const meta = { api: 1, params: {
            n: { type: "integer", default: 3 },
            b: { type: "boolean", optional: true },
            e: { type: "enum", values: ["x", "y"], default: "x" },
            p: { type: "pages", default: "2-" },
            j: { type: "json", optional: true },
            t: { type: "bytes", optional: true },
        } };
        export default async function (doc, params) {
            return { ...params, t: params.t ? Array.from(params.t) : null };
        }"#;
    let (r, _) = exec(
        &s,
        doc,
        code,
        json!({ "n": "7", "b": "yes", "j": { "k": [1] }, "t": { "$bytes": "AAEC" } }),
    );
    assert_eq!(
        r.unwrap().value,
        json!({ "n": 7, "b": true, "e": "x", "p": [1, 2, 3, 4], "j": { "k": [1] }, "t": [0, 1, 2] })
    );
    for (bad, what) in [
        (json!({ "n": "1.5" }), "parameter n: expected an integer"),
        (json!({ "e": "z" }), "parameter e: \"z\" is not one of x, y"),
        (json!({ "p": "9" }), "the document has 5 pages"),
        (json!({ "q": 1 }), "parameter q: unknown"),
    ] {
        let (r, _) = exec(&s, doc, code, bad);
        let e = r.unwrap_err().to_string();
        assert!(e.contains(what), "{e}");
    }
    let meta = read_meta("t.js", code, "t").unwrap();
    assert!(pdfi_macro::check_params(&meta, &params(json!({ "q": 1 }))).is_err());
    assert!(pdfi_macro::check_params(&meta, &params(json!({ "n": 2 }))).is_ok());
}

#[test]
fn bad_meta() {
    for (code, what) in [
        ("export const meta = { api: 2 };", "meta.api is 2"),
        ("export default 1;", "no `meta`"),
        (
            "export const meta = { api: 1, params: { a: { type: \"date\" } } };",
            "parameter a: type",
        ),
        (
            "export const meta = { api: 1, params: { a: { type: \"integer\", default: \"x\" } } };",
            "expected a number",
        ),
        ("syntax error here", "expecting ';'"),
    ] {
        let e = read_meta("t.js", code, "t").unwrap_err().to_string();
        assert!(e.contains(what), "{code}: {e}");
    }
    let m = read_meta("t.js", "export const meta = { api: 1 };", "fallback").unwrap();
    assert_eq!(m.name, "fallback");
    assert!(!m.edits);
}

#[test]
fn cancel_and_timeout_stop_loops() {
    let (s, doc) = session("model.pdf");
    let code = r#"
        export const meta = { api: 1, edits: true };
        export default async function (doc) {
            await doc.deletePages([0]);
            for (;;) {}
        }"#;
    let meta = read_meta("t.js", code, "t").unwrap();
    let opts = RunOptions {
        timeout: Some(Duration::from_millis(200)),
        ..RunOptions::default()
    };
    let (r, _) = exec_meta(&s, doc, code, &meta, json!({}), opts);
    assert!(matches!(r, Err(MacroError::Timeout)), "{r:?}");
    assert_eq!(summary(&s, doc).pages, 5);

    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let opts = RunOptions {
        cancel,
        ..RunOptions::default()
    };
    let (r, _) = exec_meta(&s, doc, code, &meta, json!({}), opts);
    assert!(matches!(r, Err(MacroError::Cancelled)), "{r:?}");
    assert!(!summary(&s, doc).edit.dirty);
}

#[test]
fn emit_files() {
    let (s, doc) = session("model.pdf");
    let code = r#"
        export const meta = { api: 1 };
        export default async function (doc, _p, ctx) {
            ctx.emit("a.txt", "héllo");
            ctx.emit("b.bin", new Uint8Array([0, 255, 128]), "application/octet-stream");
            ctx.emit("c.json", { pages: doc.pageCount });
            return ctx.input;
        }"#;
    let meta = read_meta("t.js", code, "t").unwrap();
    let opts = RunOptions {
        input: json!({ "name": "model.pdf" }),
        ..RunOptions::default()
    };
    let (r, _) = exec_meta(&s, doc, code, &meta, json!({}), opts);
    let o = r.unwrap();
    assert_eq!(o.value, json!({ "name": "model.pdf" }));
    assert_eq!(o.emitted[0].data, "héllo".as_bytes());
    assert_eq!(o.emitted[1].data, [0, 255, 128]);
    assert_eq!(
        o.emitted[1].mime.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(o.emitted[2].data, b"{\n  \"pages\": 5\n}");
}

#[test]
fn pages_content_and_inserting() {
    let (s, doc) = session("content.pdf");
    let other = s
        .open_source(
            Arc::new(MemSource::new(fixture("model.pdf"))),
            "model.pdf",
            None,
            false,
        )
        .unwrap()
        .info
        .doc;
    let code = r#"
        export const meta = { api: 1, edits: true, params: { other: { type: "pdf" } } };
        export default async function (doc, { other }) {
            const page = await doc.page(0);
            const n = await page.content.count();
            const ops = [];
            for await (const op of page.content.ops({ from: 0, to: 2 })) ops.push(op.operator);
            const text = await page.content.text();
            await page.content.insert(0, "q");
            await page.content.insert(n + 1, "Q");
            const after = await page.content.count();
            let threw = false;
            try { await other.deletePages([0]); } catch (e) { threw = e instanceof pdf.PdfiError && e.code === "forbidden"; }
            const before = doc.pageCount;
            await doc.insertPages(other, { pages: [0, 1] });
            const labels = [];
            for await (const p of doc.pages([0, before])) labels.push(p.index);
            return { n, ops, hasText: text.length > 0, after, threw, pages: doc.pageCount - before, labels };
        }"#;
    let (r, _) = exec(&s, doc, code, json!({ "other": { "$doc": other } }));
    let v = r.unwrap().value;
    assert_eq!(v["after"], v["n"].as_u64().unwrap() + 2);
    assert_eq!(v["ops"].as_array().unwrap().len(), 2);
    assert_eq!(v["hasText"], true);
    assert_eq!(v["threw"], true);
    assert_eq!(v["pages"], 2);
    assert_eq!(summary(&s, other).pages, 5);
}
