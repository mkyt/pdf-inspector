//! `pdfi macro`: list, show, create and run macros (docs/macros/).

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use pdfi_api::{DocId, NoFilesystem, SaveMode, Session, ThreadExecutor};
use pdfi_macro::params::{self, Given};
use pdfi_macro::{Level, Library, MacroError, MacroSource, Meta, Origin, RunOptions};
use pdfi_syntax::FileSource;
use serde_json::{Map, Value, json};

#[derive(Subcommand)]
pub enum MacroCmd {
    /// The bundled macros and those in the macro directories
    /// ($PDFI_MACRO_PATH, else the default one).
    List {
        #[arg(long)]
        json: bool,
    },
    /// What a macro does and the parameters it takes.
    Show {
        /// A name, or a path to a .js file.
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Write a new macro to start from, in the first macro directory.
    New {
        name: String,
        /// Write it here instead.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Replace a file that is there.
        #[arg(long)]
        force: bool,
    },
    /// Write the type definitions (pdfi.d.ts) for an editor.
    Types {
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Run a macro on one or more files.
    Run(Box<RunArgs>),
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Jsonl,
    Table,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SaveArg {
    Incremental,
    Full,
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// A name, or a path to a .js file.
    pub name: String,
    /// Input files; `-` reads paths from stdin, one per line.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// A parameter: NAME=VALUE, or NAME=@FILE for the contents of a file.
    #[arg(short, long = "param", value_name = "NAME=VALUE")]
    pub param: Vec<String>,
    /// Parameters as a JSON object (--param wins).
    #[arg(long, value_name = "FILE")]
    pub params: Option<PathBuf>,
    /// Save to this file (one input only).
    #[arg(short, long, conflicts_with_all = ["out_dir", "in_place"])]
    pub out: Option<PathBuf>,
    /// Save under the same file name in this directory; files the macro
    /// emits go here too.
    #[arg(long, conflicts_with = "in_place")]
    pub out_dir: Option<PathBuf>,
    /// Save over the input files.
    #[arg(long)]
    pub in_place: bool,
    /// Where emitted files go without --out-dir.
    #[arg(long)]
    pub emit_dir: Option<PathBuf>,
    /// How to save, instead of what the macro asks for.
    #[arg(long, value_enum)]
    pub save: Option<SaveArg>,
    /// Run, but save nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Password of the input files.
    #[arg(long)]
    pub password: Option<String>,
    /// Files run at the same time.
    #[arg(short, long, default_value_t = 1)]
    pub jobs: usize,
    /// Go on with the other files after one fails.
    #[arg(long)]
    pub keep_going: bool,
    /// Stop a file after this many seconds.
    #[arg(long, value_name = "SEC")]
    pub timeout: Option<f64>,
    #[arg(long, value_enum, default_value = "json")]
    pub format: Format,
    /// No logs on stderr (errors still).
    #[arg(short, long)]
    pub quiet: bool,
}

pub fn run(cmd: MacroCmd) -> Result<()> {
    let lib = Library::from_env();
    match cmd {
        MacroCmd::List { json } => list(&lib, json),
        MacroCmd::Show { name, json } => show(&lib, &name, json),
        MacroCmd::New { name, dir, force } => new(&lib, &name, dir, force),
        MacroCmd::Types { out } => match out {
            Some(p) => {
                std::fs::write(&p, pdfi_macro::TYPES)
                    .with_context(|| format!("writing {}", p.display()))?;
                eprintln!("wrote {}", p.display());
                Ok(())
            }
            None => {
                std::io::stdout().write_all(pdfi_macro::TYPES.as_bytes())?;
                Ok(())
            }
        },
        MacroCmd::Run(a) => run_files(&lib, *a),
    }
}

fn origin(s: &MacroSource) -> String {
    match &s.origin {
        Origin::Bundled => "bundled".into(),
        Origin::File(p) => p.display().to_string(),
    }
}

fn list(lib: &Library, as_json: bool) -> Result<()> {
    let items = lib.list();
    if as_json {
        let v: Vec<Value> = items
            .iter()
            .map(|l| {
                let mut o = json!({
                    "name": l.name(),
                    "origin": origin(&l.source),
                    "shadowed": l.shadowed,
                });
                match &l.meta {
                    Ok(m) => {
                        o["title"] = json!(m.title);
                        o["description"] = json!(m.description);
                        o["edits"] = json!(m.edits);
                    }
                    Err(e) => o["error"] = json!(e.to_string()),
                }
                o
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let w = items
        .iter()
        .map(|l| l.name().len())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:w$}  EDITS  DESCRIPTION", "NAME");
    for l in &items {
        let (edits, text) = match &l.meta {
            Ok(m) => (
                if m.edits { "yes" } else { "no" },
                if m.description.is_empty() {
                    m.title.clone()
                } else {
                    m.description.clone()
                },
            ),
            Err(e) => (
                "?",
                format!("error: {}", e.to_string().lines().next().unwrap_or("")),
            ),
        };
        let mut note = String::new();
        if !matches!(l.source.origin, Origin::Bundled) {
            note = format!(" ({})", origin(&l.source));
        }
        if l.shadowed {
            note.push_str(" (hidden by the one above)");
        }
        println!("{:w$}  {edits:5}  {text}{note}", l.name());
    }
    if lib.dirs().is_empty() {
        println!("\nNo macro directory: set PDFI_MACRO_PATH.");
    } else {
        let dirs: Vec<String> = lib.dirs().iter().map(|d| d.display().to_string()).collect();
        println!("\nMacro directories: {}", dirs.join(":"));
    }
    Ok(())
}

fn show(lib: &Library, name: &str, as_json: bool) -> Result<()> {
    let (src, m) = lib.resolve(name)?;
    if as_json {
        let mut v = serde_json::to_value(&m)?;
        v["origin"] = json!(origin(&src));
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!("{} — {}", m.name, m.title);
    if !m.description.is_empty() {
        println!("{}", m.description);
    }
    println!();
    println!("from:   {}", origin(&src));
    println!("edits:  {}", if m.edits { "yes" } else { "no" });
    if m.edits {
        println!("saves:  {}", save_text(m.save.mode()));
    }
    if m.params.is_empty() {
        println!("params: none");
        return Ok(());
    }
    println!("params:");
    for (k, p) in &m.params {
        let mut ty = p.ty.as_str().to_string();
        if let Some(vs) = &p.values {
            ty = vs.join("|");
        }
        let mut extra = Vec::new();
        if let Some(d) = &p.default {
            extra.push(format!(
                "default {}",
                d.as_str().map_or_else(|| d.to_string(), str::to_string)
            ));
        } else if p.optional {
            extra.push("optional".into());
        } else {
            extra.push("required".into());
        }
        println!("  {k} ({ty}; {})", extra.join(", "));
        if let Some(d) = &p.description {
            println!("      {d}");
        }
    }
    Ok(())
}

fn save_text(m: SaveMode) -> &'static str {
    match m {
        SaveMode::Incremental => "incremental update",
        SaveMode::Full { .. } => "full rewrite",
    }
}

fn new(lib: &Library, name: &str, dir: Option<PathBuf>, force: bool) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        bail!("{name:?}: a name has letters, digits, '.', '_' and '-'");
    }
    let dir = match dir.or_else(|| lib.dirs().first().cloned()) {
        Some(d) => d,
        None => bail!("no macro directory: give --dir or set PDFI_MACRO_PATH"),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{name}.js"));
    if path.exists() && !force {
        bail!("{} exists (--force replaces it)", path.display());
    }
    std::fs::write(&path, pdfi_macro::TEMPLATE.replace("NAME", name))?;
    let types = dir.join("pdfi.d.ts");
    if !types.exists() || force {
        std::fs::write(&types, pdfi_macro::TYPES)?;
    }
    println!("{}", path.display());
    eprintln!("run it: pdfi macro run {name} file.pdf");
    Ok(())
}

// ------------------------------------------------------------------ run

struct Plan {
    src: MacroSource,
    meta: Meta,
    file_params: Option<Map<String, Value>>,
    args: Vec<(String, Given)>,
    files: Vec<PathBuf>,
    a: RunArgs,
    cancel: Arc<AtomicBool>,
    stop: AtomicBool,
    multi: bool,
}

/// Where a file's result is saved, if anywhere.
fn target(p: &Plan, file: &Path) -> Option<PathBuf> {
    if p.a.dry_run {
        return None;
    }
    if let Some(o) = &p.a.out {
        return Some(o.clone());
    }
    if let Some(d) = &p.a.out_dir {
        return Some(d.join(file.file_name()?));
    }
    p.a.in_place.then(|| file.to_path_buf())
}

fn inputs(files: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for f in files {
        if f.as_os_str() == "-" {
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                let line = line.trim();
                if !line.is_empty() {
                    out.push(PathBuf::from(line));
                }
            }
        } else {
            out.push(f.clone());
        }
    }
    if out.is_empty() {
        bail!("no input files");
    }
    Ok(out)
}

fn run_files(lib: &Library, a: RunArgs) -> Result<()> {
    let (src, meta) = lib.resolve(&a.name)?;
    let files = inputs(&a.files)?;
    if a.out.is_some() && files.len() > 1 {
        bail!("-o takes one input file: use --out-dir or --in-place");
    }
    let saves = a.out.is_some() || a.out_dir.is_some() || a.in_place;
    if meta.edits && !saves && !a.dry_run {
        bail!(
            "{} changes the document: give -o, --out-dir or --in-place (or --dry-run)",
            meta.name
        );
    }
    let file_params = match &a.params {
        Some(p) => {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            match serde_json::from_str::<Value>(&text)
                .with_context(|| format!("{}: not JSON", p.display()))?
            {
                Value::Object(o) => Some(o),
                _ => bail!("{}: parameters are a JSON object", p.display()),
            }
        }
        None => None,
    };
    let args = a
        .param
        .iter()
        .map(|s| params::parse_arg(s))
        .collect::<Result<Vec<_>, _>>()?;
    // Check the parameters once before going through the files.
    let raw = params::collect(&meta, file_params.clone(), &args, &mut |_| Ok(0))?;
    pdfi_macro::check_params(&meta, &raw)?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let c = cancel.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async {
                if tokio::signal::ctrl_c().await.is_ok() {
                    c.store(true, Ordering::Relaxed);
                    eprintln!("\ncancelling…");
                }
            });
        });
    }
    let jobs = a.jobs.max(1).min(files.len());
    let plan = Arc::new(Plan {
        src,
        meta,
        file_params,
        args,
        multi: files.len() > 1,
        files,
        a,
        cancel,
        stop: AtomicBool::new(false),
    });

    let next = Arc::new(AtomicUsize::new(0));
    let printer = Arc::new(Mutex::new(Printer {
        format: plan.a.format,
        next: 0,
        pending: BTreeMap::new(),
        all: Vec::new(),
        failed: 0,
    }));
    let workers: Vec<_> = (0..jobs)
        .map(|_| {
            let (plan, next, printer) = (plan.clone(), next.clone(), printer.clone());
            std::thread::Builder::new()
                .stack_size(16 << 20)
                .spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= plan.files.len()
                            || plan.stop.load(Ordering::Relaxed)
                            || plan.cancel.load(Ordering::Relaxed)
                        {
                            break;
                        }
                        let r = one(&plan, &plan.files[i]);
                        if !r["ok"].as_bool().unwrap_or(false) && !plan.a.keep_going {
                            plan.stop.store(true, Ordering::Relaxed);
                        }
                        printer.lock().unwrap_or_else(|e| e.into_inner()).add(i, r);
                    }
                })
                .expect("spawning a worker")
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    let mut p = printer.lock().unwrap_or_else(|e| e.into_inner());
    p.finish();
    let skipped = plan.files.len() - p.all.len();
    if skipped > 0 {
        eprintln!("{skipped} file(s) not run after the first failure (--keep-going runs them)");
    }
    if p.failed > 0 || skipped > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Prints results in input order as they arrive.
struct Printer {
    format: Format,
    next: usize,
    pending: BTreeMap<usize, Value>,
    all: Vec<Value>,
    failed: usize,
}

impl Printer {
    fn add(&mut self, i: usize, r: Value) {
        if !r["ok"].as_bool().unwrap_or(false) {
            self.failed += 1;
        }
        self.pending.insert(i, r);
        while let Some(r) = self.pending.remove(&self.next) {
            self.next += 1;
            match self.format {
                Format::Jsonl => println!("{}", public(&r)),
                Format::Table => print_table(&r),
                Format::Json => {}
            }
            self.all.push(r);
        }
    }

    fn finish(&mut self) {
        // Results after a gap (a file not run) still count.
        let rest: Vec<Value> = std::mem::take(&mut self.pending).into_values().collect();
        for r in rest {
            match self.format {
                Format::Jsonl => println!("{}", public(&r)),
                Format::Table => print_table(&r),
                Format::Json => {}
            }
            self.all.push(r);
        }
        if self.format == Format::Json {
            let all: Vec<Value> = self.all.iter().map(public).collect();
            println!("{}", serde_json::to_string_pretty(&all).unwrap());
        }
    }
}

/// A result without what only the table uses.
fn public(r: &Value) -> Value {
    let mut r = r.clone();
    if let Some(o) = r.as_object_mut() {
        o.remove("columns");
    }
    r
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn print_table(r: &Value) {
    println!("== {}", cell(&r["file"]));
    if !r["ok"].as_bool().unwrap_or(false) {
        println!("error: {}", cell(&r["error"]));
        return;
    }
    let v = &r["value"];
    let rows = v.as_array().filter(|rows| {
        !rows.is_empty()
            && rows.iter().all(|x| {
                x.as_object()
                    .is_some_and(|o| o.values().all(|c| !c.is_object() && !c.is_array()))
            })
    });
    match rows {
        Some(rows) => {
            let mut cols: Vec<String> = r["columns"]
                .as_array()
                .map(|c| {
                    c.iter()
                        .filter_map(|c| c.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            for row in rows {
                for k in row.as_object().unwrap().keys() {
                    if !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
            }
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|row| cols.iter().map(|c| cell(&row[c])).collect())
                .collect();
            let widths: Vec<usize> = cols
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    cells
                        .iter()
                        .map(|r| r[i].chars().count())
                        .chain([c.chars().count()])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let line = |vals: &[String]| {
                let s: Vec<String> = vals
                    .iter()
                    .zip(&widths)
                    .map(|(v, w)| format!("{v}{}", " ".repeat(w - v.chars().count())))
                    .collect();
                println!("{}", s.join("  ").trim_end());
            };
            line(&cols);
            for r in &cells {
                line(r);
            }
        }
        None if !v.is_null() => println!("{}", serde_json::to_string_pretty(v).unwrap()),
        None => {}
    }
    if let Some(p) = r["saved"].as_str() {
        println!("saved {p}");
    }
}

fn emit_name(plan: &Plan, file: &Path, name: &str) -> String {
    if !plan.multi {
        return name.to_string();
    }
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{stem}-{name}")
}

/// Run on one file; the result record.
fn one(plan: &Plan, file: &Path) -> Value {
    let shown = file.display().to_string();
    let mut rec = json!({
        "file": shown,
        "ok": false,
        "value": null,
        "error": null,
        "saved": null,
        "emitted": [],
    });
    match one_inner(plan, file, &mut rec) {
        Ok(()) => rec["ok"] = json!(true),
        Err(e) => {
            let msg = format!("{e:#}");
            eprintln!("{shown}: {msg}");
            rec["error"] = json!(msg);
        }
    }
    rec
}

fn open(s: &Session, path: &Path, password: Option<String>) -> Result<DocId> {
    let src = FileSource::open(path).with_context(|| format!("opening {}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(s.open_source(Arc::new(src), name, password, false)?
        .info
        .doc)
}

fn one_inner(plan: &Plan, file: &Path, rec: &mut Value) -> Result<()> {
    let session = Arc::new(Session::new("cli", NoFilesystem, ThreadExecutor));
    let doc = open(&session, file, plan.a.password.clone())?;
    let raw = params::collect(&plan.meta, plan.file_params.clone(), &plan.args, &mut |p| {
        open(&session, p, None).map_err(|e| MacroError::Host(format!("{e:#}")))
    })?;
    let prefix = if plan.multi {
        format!("[{}] ", file.display())
    } else {
        String::new()
    };
    let quiet = plan.a.quiet;
    let progress_shown = Arc::new(AtomicBool::new(false));
    let show_progress = !quiet && !plan.multi && std::io::stderr().is_terminal();
    let p2 = prefix.clone();
    let shown = progress_shown.clone();
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    let opts = RunOptions {
        cancel: plan.cancel.clone(),
        timeout: plan.a.timeout.map(Duration::from_secs_f64),
        host: "cli".into(),
        input: json!({
            "name": file.file_name().map(|n| n.to_string_lossy().into_owned()),
            "size": size,
        }),
        log: Box::new(move |level, text| {
            if quiet && level != Level::Error {
                return;
            }
            let tag = match level {
                Level::Info => "",
                Level::Warn => "warning: ",
                Level::Error => "error: ",
            };
            let mut e = std::io::stderr().lock();
            if shown.swap(false, Ordering::Relaxed) {
                let _ = write!(e, "\r\x1b[K");
            }
            for line in text.lines() {
                let _ = writeln!(e, "{prefix}{tag}{line}");
            }
        }),
        progress: Box::new(move |done, total, label| {
            if !show_progress {
                return;
            }
            let label = label.map(|l| format!(" {l}")).unwrap_or_default();
            eprint!("\r\x1b[K{p2}{done}/{total}{label}");
            progress_shown.store(done < total, Ordering::Relaxed);
            if done >= total {
                eprint!("\r\x1b[K");
            }
        }),
    };
    let outcome = pdfi_macro::run(
        &session,
        doc,
        &plan.src.file,
        &plan.src.code,
        &plan.meta,
        &raw,
        opts,
    );
    if show_progress {
        eprint!("\r\x1b[K");
    }
    let outcome = outcome?;
    rec["value"] = outcome.value;
    if let Some(c) = outcome.columns {
        rec["columns"] = json!(c);
    }

    // Save.
    let status = match session
        .dispatch(pdfi_api::Request::Summary { doc })
        .response
    {
        pdfi_api::Response::Ok(pdfi_api::ResponseBody::Doc(d)) => d.edit,
        r => bail!("{r:?}"),
    };
    let wants = plan.meta.edits || status.dirty;
    match target(plan, file) {
        Some(out) if wants => {
            let mut mode = match plan.a.save {
                Some(SaveArg::Incremental) => SaveMode::Incremental,
                Some(SaveArg::Full) => SaveMode::Full {
                    xref_stream: false,
                    object_streams: false,
                    compress: false,
                    remove_unused: false,
                },
                None => plan.meta.save.mode(),
            };
            if mode == SaveMode::Incremental
                && let Some(why) = &status.no_incremental
            {
                if !quiet {
                    eprintln!("{}: {why}", file.display());
                }
                mode = SaveMode::Full {
                    xref_stream: false,
                    object_streams: false,
                    compress: false,
                    remove_unused: false,
                };
            }
            save(&session, doc, mode, &out)?;
            rec["saved"] = json!(out.display().to_string());
        }
        None if status.dirty && !plan.a.dry_run && !quiet => {
            eprintln!(
                "{}: the macro changed the document, but nothing says where to save it (-o, --out-dir, --in-place)",
                file.display()
            );
        }
        _ => {}
    }

    // Emitted files.
    let dir = plan.a.out_dir.as_ref().or(plan.a.emit_dir.as_ref());
    let mut emitted = Vec::new();
    for e in &outcome.emitted {
        let name = emit_name(plan, file, &e.name);
        let path = match (dir, plan.a.dry_run) {
            (Some(d), false) => {
                std::fs::create_dir_all(d)?;
                let p = d.join(&name);
                std::fs::write(&p, &e.data).with_context(|| format!("writing {}", p.display()))?;
                Some(p.display().to_string())
            }
            _ => {
                if !quiet && !plan.a.dry_run {
                    eprintln!(
                        "{}: {name} not written: give --out-dir or --emit-dir",
                        file.display()
                    );
                }
                None
            }
        };
        emitted.push(json!({ "name": name, "path": path, "bytes": e.data.len() }));
    }
    rec["emitted"] = Value::Array(emitted);
    Ok(())
}

/// Write the document next to `out` and rename it over `out`.
fn save(s: &Session, doc: DocId, mode: SaveMode, out: &Path) -> Result<()> {
    let dir = out
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out.pdf".into());
    let tmp = dir.join(format!(".{name}.pdfi-{}.tmp", std::process::id()));
    let r = (|| -> Result<()> {
        let mut st = s.save_stream(doc, mode)?;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        while let Some(c) = st.next_chunk()? {
            f.write_all(&c)?;
        }
        f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        for w in st.result().warnings {
            eprintln!("{}: {w}", out.display());
        }
        std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}
