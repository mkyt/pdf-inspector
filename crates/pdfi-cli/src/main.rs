use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pdfi_syntax::xref::{XrefEntry, XrefKind};
use pdfi_syntax::{
    FileSource, ObjId, ObjLocation, Object, OpenOptions, PdfFile, Recovery, Severity, write,
};

mod content;
mod edit;
mod font;
mod model;

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
    /// Validate against the Arlington PDF Model, following typed references
    /// from the trailer (or from one object).
    Validate {
        #[command(flatten)]
        f: FileArgs,
        /// Start at this object number instead of the trailer.
        #[arg(long)]
        obj: Option<u32>,
        /// Include informational issues (unknown keys, deprecations).
        #[arg(short, long)]
        verbose: bool,
    },
    /// List pages: label, object, visible size, rotation, annotations.
    Pages {
        #[command(flatten)]
        f: FileArgs,
        /// First page (1-based).
        #[arg(long, default_value_t = 1)]
        from: u64,
        #[arg(long)]
        count: Option<u64>,
    },
    /// One page (1-based): inherited attributes, boxes, resources, contents,
    /// annotations.
    Page {
        #[command(flatten)]
        f: FileArgs,
        number: u64,
    },
    /// Print the outline (bookmarks).
    Outline(FileArgs),
    /// Info dictionary, XMP metadata and what the catalog declares.
    Meta(FileArgs),
    /// Fonts and images with the pages using them; with --refs, the objects
    /// referring to one object.
    Inventory {
        #[command(flatten)]
        f: FileArgs,
        #[arg(long)]
        refs: Option<u32>,
    },
    /// Operations of a page's content (1-based page number) or of a stream
    /// (--obj): operands, decoded text, what they paint and their problems.
    Ops {
        #[command(flatten)]
        f: FileArgs,
        page: Option<u64>,
        /// A stream object (form XObject, pattern, glyph procedure, ...).
        #[arg(long)]
        obj: Option<u32>,
        /// First operation (0-based).
        #[arg(long, default_value_t = 0)]
        from: usize,
        #[arg(long)]
        count: Option<usize>,
        /// Print the graphics state before and after this operation.
        #[arg(long)]
        state: Option<usize>,
        /// Print the problems only.
        #[arg(long)]
        issues: bool,
    },
    /// A font: its dictionaries, program and problems; with --glyphs, the
    /// code → glyph → text table.
    Font {
        #[command(flatten)]
        f: FileArgs,
        num: u32,
        #[arg(long)]
        glyphs: bool,
    },
    /// Render a page (1-based page number) or a stream (--obj) to PNG.
    Render {
        #[command(flatten)]
        f: FileArgs,
        page: Option<u64>,
        #[arg(long)]
        obj: Option<u32>,
        /// Output PNG file.
        #[arg(short, long)]
        out: PathBuf,
        /// Resolution (72 = one pixel per point).
        #[arg(long, default_value_t = 144.0)]
        dpi: f64,
        /// Stop after this operation (0-based).
        #[arg(long)]
        until: Option<usize>,
        /// Only this region of the page's bitmap: x,y,width,height (pixels).
        #[arg(long, value_delimiter = ',')]
        tile: Vec<u32>,
        /// Draw only these operations as a highlight layer (comma separated).
        #[arg(long, value_delimiter = ',')]
        highlight: Vec<u32>,
        /// Also print the operations painting at this view point (points
        /// from the top left), e.g. --hit 100,200.
        #[arg(long, value_delimiter = ',')]
        hit: Vec<f64>,
    },
    /// Cut a region of a page (or a stream, --obj) out into a one-page PDF
    /// with only the operations painting there.
    Extract {
        #[command(flatten)]
        f: FileArgs,
        page: Option<u64>,
        #[arg(long)]
        obj: Option<u32>,
        /// The region in the content's space (default user space for
        /// pages): x0,y0,x1,y1.
        #[arg(long, value_delimiter = ',', required = true)]
        rect: Vec<f64>,
        /// Also take what only overlaps the region.
        #[arg(long)]
        overlaps: bool,
        /// Shrink the page to what is kept.
        #[arg(long)]
        fit: bool,
        /// Output PDF file.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Export a page (or a stream, --obj) as SVG to edit in a vector
    /// graphics program; with --rect, only what paints in that region.
    Svg {
        #[command(flatten)]
        f: FileArgs,
        page: Option<u64>,
        #[arg(long)]
        obj: Option<u32>,
        /// Only this region in the content's space: x0,y0,x1,y1.
        #[arg(long, value_delimiter = ',')]
        rect: Vec<f64>,
        /// Also take what only overlaps the region.
        #[arg(long)]
        overlaps: bool,
        /// Shrink the view to what is kept.
        #[arg(long)]
        fit: bool,
        /// Text as glyph outlines instead of <text>.
        #[arg(long)]
        outlines: bool,
        /// Output SVG file.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Edit and save: Info entries, object entries, pages. Edits apply in
    /// the order of the options' kinds as listed here; the result is an
    /// incremental update unless a full rewrite is asked for.
    Edit(Box<edit::EditArgs>),
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
        Cmd::Validate { f, obj, verbose } => validate(&open(&f)?, obj, verbose),
        Cmd::Pages { f, from, count } => model::pages(&open(&f)?, from.saturating_sub(1), count),
        Cmd::Page { f, number } => model::page(&open(&f)?, number),
        Cmd::Outline(f) => model::outline(&open(&f)?),
        Cmd::Meta(f) => model::meta(&open(&f)?),
        Cmd::Inventory { f, refs } => model::inventory(&open(&f)?, refs),
        Cmd::Ops {
            f,
            page,
            obj,
            from,
            count,
            state,
            issues,
        } => content::ops(&open(&f)?, page, obj, from, count, state, issues),
        Cmd::Font { f, num, glyphs } => font::font(&open(&f)?, num, glyphs),
        Cmd::Render {
            f,
            page,
            obj,
            out,
            dpi,
            until,
            tile,
            highlight,
            hit,
        } => content::render(
            &open(&f)?,
            page,
            obj,
            &out,
            dpi,
            until,
            &tile,
            highlight,
            &hit,
        ),
        Cmd::Extract {
            f,
            page,
            obj,
            rect,
            overlaps,
            fit,
            out,
        } => content::extract(&open(&f)?, page, obj, &rect, overlaps, fit, &out),
        Cmd::Svg {
            f,
            page,
            obj,
            rect,
            overlaps,
            fit,
            outlines,
            out,
        } => content::svg(&open(&f)?, page, obj, &rect, overlaps, fit, outlines, &out),
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
                ObjLocation::Edited => println!("% {} (edited)", o.id),
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
        Cmd::Edit(a) => edit::run(*a),
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
    println!(
        "pages:      {}",
        pdfi_model::DocModel::new().on(file).page_count()
    );
    print_diags(file, false, None);
    Ok(())
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
        XrefEntry::Edited { generation } => println!("{num:>8} {generation:>5} edited"),
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
    let pages = pdfi_model::DocModel::new().on(&file).page_count();
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

fn validate(file: &PdfFile, obj: Option<u32>, verbose: bool) -> Result<()> {
    use pdfi_schema::{DocSchema, Walk, path_text};
    let t0 = Instant::now();
    let schema = DocSchema::new();
    let ck = schema.checker(file);
    let mut walk = match obj {
        None => Walk::document(&ck),
        Some(num) => {
            let id = file.get_num(num)?.id;
            let (ty, _) = ck
                .type_of(id)
                .with_context(|| format!("type of object {id} is unknown"))?;
            Walk::object(id, ty)
        }
    };
    let mut issues = Vec::new();
    while walk.step(&ck, 256, &mut issues) {}
    let mut counts = [0usize; 3];
    for i in &issues {
        counts[i.severity as usize] += 1;
        if i.severity == Severity::Info && !verbose {
            continue;
        }
        let at = match i.obj {
            Some(id) => format!("{id} obj"),
            None => "trailer".into(),
        };
        let ty = i
            .obj
            .and_then(|id| ck.known_type(id))
            .map(|t| t.name())
            .unwrap_or_default();
        println!(
            "{:<7} {:<15} {at}{} ({ty}): {}",
            format!("{:?}", i.severity).to_lowercase(),
            format!("{:?}", i.code),
            path_text(&i.path),
            i.message
        );
    }
    println!(
        "{} objects validated, {} typed; {} errors, {} warnings, {} info ({:.2?})",
        walk.done,
        schema.known(),
        counts[2],
        counts[1],
        counts[0],
        t0.elapsed()
    );
    Ok(())
}
