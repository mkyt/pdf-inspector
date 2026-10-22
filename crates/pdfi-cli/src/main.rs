use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pdfi_syntax::xref::{XrefEntry, XrefKind};
use pdfi_syntax::{
    DictExt, FileSource, ObjId, ObjLocation, Object, OpenOptions, PdfFile, Recovery, Severity,
    write,
};

#[derive(Parser)]
#[command(name = "pdfi", about = "Inspect PDF internals")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
struct FileArgs {
    file: PathBuf,
    #[arg(long)]
    password: Option<String>,
    /// Ignore cross-reference data and reconstruct it by scanning.
    #[arg(long)]
    recover: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Summary: version, revisions, trailer, encryption, page count.
    Info(FileArgs),
    /// List cross-reference entries.
    Xref {
        #[command(flatten)]
        f: FileArgs,
        /// Show the entries of one revision (0 = newest) instead of the merged table.
        #[arg(long)]
        rev: Option<usize>,
    },
    /// Print an object.
    Obj {
        #[command(flatten)]
        f: FileArgs,
        num: u32,
        generation: Option<u16>,
    },
    /// Write stream data to stdout or a file.
    Stream {
        #[command(flatten)]
        f: FileArgs,
        num: u32,
        /// Raw data as stored in the file.
        #[arg(long)]
        raw: bool,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Load every object (and optionally decode every stream); report problems.
    Check {
        #[command(flatten)]
        f: FileArgs,
        #[arg(long)]
        decode: bool,
        /// Print every diagnostic, not only a summary.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Rewrite as a plain file (object streams expanded, decrypted).
    Rewrite {
        #[command(flatten)]
        f: FileArgs,
        out: PathBuf,
        /// Re-serialize every object instead of copying unchanged bytes.
        #[arg(long)]
        reserialize: bool,
    },
    /// Serve the web UI on 127.0.0.1 and open it in the browser.
    Serve {
        /// Files to open; they may be opened from the UI as well.
        files: Vec<PathBuf>,
        /// Directory the UI may browse and open files from (repeatable).
        /// Defaults to the current directory when no files are given.
        #[arg(long)]
        root: Vec<PathBuf>,
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Do not open a browser.
        #[arg(long)]
        no_open: bool,
        /// Extra allowed WebSocket origin, e.g. http://localhost:5173 for the
        /// Vite dev server (repeatable).
        #[arg(long)]
        dev_origin: Vec<String>,
    },
}

fn open(a: &FileArgs) -> Result<PdfFile> {
    let src = FileSource::open(&a.file).with_context(|| format!("opening {}", a.file.display()))?;
    let opts = OpenOptions {
        password: a.password.as_ref().map(|p| p.as_bytes().to_vec()),
        recovery: if a.recover {
            Recovery::Force
        } else {
            Recovery::Auto
        },
        ..Default::default()
    };
    Ok(PdfFile::open(Arc::new(src), opts)?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info(a) => info(&open(&a)?),
        Cmd::Xref { f, rev } => xref(&open(&f)?, rev),
        Cmd::Obj { f, num, generation } => {
            let file = open(&f)?;
            let o = match generation {
                Some(g) => file.get(ObjId::new(num, g))?,
                None => file.get_num(num)?,
            };
            match o.location {
                ObjLocation::File(s) => println!("% {} at bytes {}..{}", o.id, s.start, s.end),
                ObjLocation::ObjStm { stream, index } => {
                    println!("% {} in object stream {stream} (index {index})", o.id)
                }
            }
            println!("{}", write::pretty(&o.node.obj));
            if let Object::Stream(s) = &o.node.obj {
                let d = file.decode_stream(o.id, s, None);
                println!("% stream: {} bytes decoded{}", d.data.len(), stop_note(&d));
                for e in &d.errors {
                    println!("% decode error: {e}");
                }
            }
            print_diags(&file, false, Some(o.id));
            Ok(())
        }
        Cmd::Stream { f, num, raw, out } => {
            let file = open(&f)?;
            let o = file.get_num(num)?;
            let Object::Stream(s) = &o.node.obj else {
                bail!("object {num} is not a stream")
            };
            let mut sink: Box<dyn Write> = match out {
                Some(p) => Box::new(std::fs::File::create(p)?),
                None => Box::new(std::io::stdout().lock()),
            };
            if raw {
                std::io::copy(&mut file.raw_reader(s), &mut sink)?;
            } else {
                let (mut r, chain) = file.decoded_reader(o.id, s);
                std::io::copy(&mut r, &mut sink)?;
                for e in r.errors() {
                    eprintln!("decode error: {e}");
                }
                if let Some(n) = chain.stopped_at {
                    eprintln!("note: data is still {} encoded", n.as_str_lossy());
                }
            }
            Ok(())
        }
        Cmd::Check { f, decode, verbose } => check(&f, decode, verbose),
        Cmd::Serve {
            files,
            mut root,
            port,
            no_open,
            dev_origin,
        } => {
            if files.is_empty() && root.is_empty() {
                root.push(std::env::current_dir()?);
            }
            serve(
                pdfi_server::ServeOptions {
                    files,
                    roots: root,
                    port,
                    dev_origins: dev_origin,
                },
                !no_open,
            )
        }
        Cmd::Rewrite {
            f,
            out,
            reserialize,
        } => {
            let file = open(&f)?;
            let t = Instant::now();
            let sink = std::fs::File::create(&out)?;
            let rep = write::rewrite(
                &file,
                sink,
                &write::RewriteOptions {
                    copy_unchanged: !reserialize,
                },
            )?;
            println!(
                "wrote {} objects to {} in {:.2?}",
                rep.objects,
                out.display(),
                t.elapsed()
            );
            for (id, e) in rep.failed {
                println!("  skipped {id}: {e}");
            }
            Ok(())
        }
    }
}

fn serve(opts: pdfi_server::ServeOptions, open_browser: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let server = pdfi_server::Server::bind(opts).await?;
        let url = server.url();
        eprintln!("pdfi: serving on {url}");
        eprintln!("pdfi: press Ctrl-C to stop");
        if open_browser {
            open_url(&url);
        }
        server
            .run(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    })
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let cmd = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = cmd {
        eprintln!("pdfi: could not open a browser ({e}); open the URL above");
    }
}

fn stop_note(d: &pdfi_syntax::DecodedStream) -> String {
    match &d.stopped_at {
        Some(n) => format!(" (left {} encoded)", n.as_str_lossy()),
        None => String::new(),
    }
}

fn info(file: &PdfFile) -> Result<()> {
    let h = file.header();
    println!("size:       {} bytes", file.source().len());
    println!(
        "version:    {}",
        h.version.as_deref().unwrap_or("(no header)")
    );
    if h.offset > 0 {
        println!("header at:  {}", h.offset);
    }
    println!(
        "objects:    {} live / {} slots",
        file.xref().live_ids().count(),
        file.xref().size()
    );
    println!("recovered:  {}", file.is_recovered());
    println!("revisions:  {}", file.revisions().len());
    for (i, r) in file.revisions().iter().enumerate() {
        println!(
            "  [{i}] {:<9} at {:>10}  entries {:>7}{}",
            format!("{:?}", r.kind),
            r.offset,
            r.entries.len() + r.stream_entries.len(),
            r.xref_stream
                .map(|id| format!("  (stream {id})"))
                .unwrap_or_default()
        );
    }
    println!(
        "trailer:    {}",
        write::pretty(&Object::Dict(file.trailer().clone())).replace('\n', "\n            ")
    );
    if let Some(sec) = file.security() {
        let i = &sec.info;
        println!(
            "encryption: V{} R{} {}-bit stream={:?} string={:?} auth={:?} P={}",
            i.v, i.r, i.key_bits, i.stream_method, i.string_method, sec.auth, i.permissions
        );
    }
    match count_pages(file) {
        Ok(n) => println!("pages:      {n}"),
        Err(e) => println!("pages:      error: {e}"),
    }
    print_diags(file, false, None);
    Ok(())
}

fn count_pages(file: &PdfFile) -> Result<usize> {
    let root = file.resolve(file.trailer().get_key("Root").context("no /Root")?)?;
    let root = root.as_dict().context("/Root is not a dictionary")?;
    let mut stack = vec![root.get_key("Pages").context("no /Pages")?.clone()];
    let mut seen = HashSet::new();
    let mut n = 0;
    while let Some(o) = stack.pop() {
        if let Object::Ref(id) = o {
            if !seen.insert(id) {
                continue;
            }
        }
        let o = file.resolve(&o)?;
        let Some(d) = o.as_dict() else { continue };
        match d.get_key("Kids") {
            Some(kids) if !d.type_is("Page") => {
                if let Object::Array(a) = file.resolve(kids)? {
                    stack.extend(a.into_iter().rev().map(|k| k.obj));
                }
            }
            _ => n += 1,
        }
    }
    Ok(n)
}

fn xref(file: &PdfFile, rev: Option<usize>) -> Result<()> {
    let print = |num: u32, e: XrefEntry| match e {
        XrefEntry::Free { next, generation } => {
            println!("{num:>8} {generation:>5} free  next {next}")
        }
        XrefEntry::InUse { offset, generation } => {
            println!("{num:>8} {generation:>5} in-use offset {offset}")
        }
        XrefEntry::Compressed { stream, index } => {
            println!("{num:>8}     0 compressed in {stream} [{index}]")
        }
    };
    match rev {
        Some(i) => {
            let r = file.revisions().get(i).context("no such revision")?;
            if r.kind == XrefKind::Hybrid {
                println!("% table part");
            }
            r.entries.iter().for_each(|&(n, e)| print(n, e));
            if !r.stream_entries.is_empty() {
                println!("% stream part");
                r.stream_entries.iter().for_each(|&(n, e)| print(n, e));
            }
        }
        None => file.xref().iter().for_each(|(n, e)| print(n, e)),
    }
    Ok(())
}

fn check(a: &FileArgs, decode: bool, verbose: bool) -> Result<()> {
    let t = Instant::now();
    let file = open(a)?;
    let t_open = t.elapsed();
    let ids: Vec<ObjId> = file.xref().live_ids().collect();
    let (mut ok, mut failed, mut missing, mut streams, mut decoded_bytes, mut decode_errors) =
        (0, 0, 0, 0, 0u64, 0);
    for id in &ids {
        match file.get(*id) {
            Ok(o) => {
                ok += 1;
                if let (Object::Stream(s), true) = (&o.node.obj, decode) {
                    streams += 1;
                    let (mut r, _) = file.decoded_reader(o.id, s);
                    let mut buf = vec![0; 1 << 16];
                    loop {
                        match r.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => decoded_bytes += n as u64,
                            Err(e) => {
                                decode_errors += 1;
                                if verbose {
                                    println!("  {id}: {e}");
                                }
                                break;
                            }
                        }
                    }
                    if !r.errors().is_empty() {
                        decode_errors += 1;
                        if verbose {
                            println!("  {id}: decode: {}", r.errors().join("; "));
                        }
                    }
                }
            }
            Err(pdfi_syntax::Error::Missing(_)) => missing += 1,
            Err(e) => {
                failed += 1;
                if verbose {
                    println!("  {id}: {e}");
                }
            }
        }
    }
    let pages = count_pages(&file)
        .map(|n| n.to_string())
        .unwrap_or_else(|e| format!("error: {e}"));
    println!(
        "{}: open {:.2?}, total {:.2?}; objects ok {ok} failed {failed} missing {missing}; pages {pages}{}",
        a.file.display(),
        t_open,
        t.elapsed(),
        if decode {
            format!("; streams {streams} decoded {decoded_bytes} bytes, errors {decode_errors}")
        } else {
            String::new()
        }
    );
    print_diags(&file, verbose, None);
    Ok(())
}

fn print_diags(file: &PdfFile, all: bool, only: Option<ObjId>) {
    let diags: Vec<_> = file
        .diagnostics()
        .into_iter()
        .filter(|d| only.is_none() || d.obj == only)
        .collect();
    if diags.is_empty() {
        return;
    }
    let errors = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    println!("diagnostics: {} ({errors} errors)", diags.len());
    let mut by_code = std::collections::BTreeMap::new();
    for d in &diags {
        *by_code.entry(format!("{:?}", d.code)).or_insert(0) += 1;
    }
    if all || diags.len() <= 10 {
        for d in &diags {
            let obj = d.obj.map(|o| format!(" [{o}]")).unwrap_or_default();
            let at = d.span.map(|s| format!(" @{}", s.start)).unwrap_or_default();
            println!("  {:?} {:?}{obj}{at}: {}", d.severity, d.code, d.message);
        }
    } else {
        for (c, n) in by_code {
            println!("  {c}: {n}");
        }
    }
}
