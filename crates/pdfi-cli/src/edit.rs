//! `pdfi edit`: apply edits given on the command line and save.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use pdfi_edit::{Command, FullOptions, NodeAt, Root, SaveMode, Saver, Tx, parse_value};
use pdfi_syntax::{Name, ObjId, PdfFile};

#[derive(clap::Args)]
pub struct EditArgs {
    #[command(flatten)]
    pub f: crate::FileArgs,
    #[arg(short, long)]
    pub out: PathBuf,
    /// Set an Info entry (`Title=Text`, dates as ISO 8601); the matching
    /// XMP property follows.
    #[arg(long, value_name = "KEY=TEXT")]
    pub info: Vec<String>,
    /// Set a dictionary entry of an object, the value in PDF syntax:
    /// `12:/Rotate=90`, `trailer:/Info=7 0 R`.
    #[arg(long, value_name = "OBJ:/KEY=VALUE")]
    pub set: Vec<String>,
    /// Remove a dictionary entry: `12:/Annots`.
    #[arg(long, value_name = "OBJ:/KEY")]
    pub remove: Vec<String>,
    /// Delete an object.
    #[arg(long, value_name = "NUM")]
    pub delete_object: Vec<u32>,
    /// Delete pages (1-based): `2,5-7`.
    #[arg(long, value_name = "PAGES")]
    pub delete_pages: Option<String>,
    /// Rotate pages: `1-3:90`.
    #[arg(long, value_name = "PAGES:DEGREES")]
    pub rotate: Vec<String>,
    /// Move pages to a position (1-based, in the result): `5-6:1`.
    #[arg(long = "move", value_name = "PAGES:TO")]
    pub move_pages: Vec<String>,
    /// Append the pages of another file.
    #[arg(long, value_name = "FILE")]
    pub append: Vec<PathBuf>,
    /// Insert a blank page before page N (`N`, `N:595x842`).
    #[arg(long, value_name = "N[:WxH]")]
    pub blank: Vec<String>,
    /// Rewrite the whole file instead of appending an update.
    #[arg(long)]
    pub full: bool,
    /// Full rewrite: a cross-reference stream.
    #[arg(long)]
    pub xref_stream: bool,
    /// Full rewrite: object streams.
    #[arg(long)]
    pub object_streams: bool,
    /// Full rewrite: compress unfiltered streams.
    #[arg(long)]
    pub compress: bool,
    /// Full rewrite: leave out unreachable objects.
    #[arg(long)]
    pub gc: bool,
}

/// `2,5-7` → 0-based indices.
fn pages(s: &str) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let (a, b): (u64, u64) = (a.trim().parse()?, b.trim().parse()?);
        if a == 0 || b < a {
            bail!("bad page range {part:?}");
        }
        out.extend(a - 1..b);
    }
    Ok(out)
}

fn split<'a>(s: &'a str, sep: char, what: &str) -> Result<(&'a str, &'a str)> {
    s.split_once(sep)
        .with_context(|| format!("{s:?}: expected {what}"))
}

fn root(s: &str) -> Result<Root> {
    Ok(if s == "trailer" {
        Root::Trailer
    } else {
        Root::Object(ObjId::new(s.parse().context("object number")?, 0))
    })
}

pub fn run(a: EditArgs) -> Result<()> {
    let file = crate::open(&a.f)?;
    let others: Vec<PdfFile> = a
        .append
        .iter()
        .map(|p| {
            crate::open(&crate::FileArgs {
                file: p.clone(),
                password: None,
                recover: false,
            })
        })
        .collect::<Result<_>>()?;
    let mut cmds = Vec::new();
    for kv in &a.info {
        let (k, v) = split(kv, '=', "KEY=TEXT")?;
        cmds.push(Command::SetInfo {
            key: k.trim_start_matches('/').into(),
            value: Some(v.into()),
            sync_xmp: true,
        });
    }
    for s in &a.set {
        let (obj, rest) = split(s, ':', "OBJ:/KEY=VALUE")?;
        let (k, v) = split(rest, '=', "OBJ:/KEY=VALUE")?;
        cmds.push(Command::SetEntry {
            at: NodeAt {
                root: root(obj)?,
                path: Vec::new(),
            },
            key: Name::from(k.trim().trim_start_matches('/')),
            value: parse_value(v)?,
        });
    }
    for s in &a.remove {
        let (obj, k) = split(s, ':', "OBJ:/KEY")?;
        cmds.push(Command::RemoveEntry {
            at: NodeAt {
                root: root(obj)?,
                path: Vec::new(),
            },
            key: Name::from(k.trim().trim_start_matches('/')),
        });
    }
    for &n in &a.delete_object {
        let id = file.get_num(n)?.id;
        cmds.push(Command::DeleteObject { id });
    }
    for s in &a.rotate {
        let (p, by) = split(s, ':', "PAGES:DEGREES")?;
        cmds.push(Command::RotatePages {
            pages: pages(p)?,
            by: by.parse()?,
        });
    }
    for s in &a.move_pages {
        let (p, to) = split(s, ':', "PAGES:TO")?;
        let to: u64 = to.parse()?;
        cmds.push(Command::MovePages {
            pages: pages(p)?,
            to: to.saturating_sub(1),
        });
    }
    for s in &a.blank {
        let (n, size) = s.split_once(':').unwrap_or((s, "612x792"));
        let (w, h) = split(size, 'x', "WxH")?;
        cmds.push(Command::InsertBlankPage {
            at: n.parse::<u64>()?.saturating_sub(1),
            width: w.parse()?,
            height: h.parse()?,
        });
    }
    if let Some(p) = &a.delete_pages {
        cmds.push(Command::DeletePages { pages: pages(p)? });
    }
    for o in &others {
        let n = pdfi_model::DocModel::new().on(o).page_count();
        cmds.push(Command::InsertPages {
            at: u64::MAX,
            from: Some(o),
            pages: (0..n).collect(),
        });
    }
    let t = Instant::now();
    let (edits, applied) = Tx::apply_all(&file, cmds)?;
    for n in &applied.notes {
        println!("note: {n}");
    }
    let edited = file.with_edits(edits);
    let mode = if a.full || a.xref_stream || a.object_streams || a.compress || a.gc {
        SaveMode::Full(FullOptions {
            xref_stream: a.xref_stream,
            object_streams: a.object_streams,
            compress: a.compress,
            remove_unused: a.gc,
        })
    } else {
        SaveMode::Incremental
    };
    let mut saver = Saver::new(&edited, mode)?;
    let mut out = std::io::BufWriter::new(
        std::fs::File::create(&a.out).with_context(|| format!("creating {}", a.out.display()))?,
    );
    let rep = saver.write_all(&mut out)?;
    std::io::Write::flush(&mut out)?;
    println!(
        "{} object(s) changed; wrote {} bytes ({} objects) to {} in {:.2?}",
        applied.changed.len(),
        rep.bytes,
        rep.objects,
        a.out.display(),
        t.elapsed()
    );
    for w in rep.warnings {
        println!("warning: {w}");
    }
    Ok(())
}
