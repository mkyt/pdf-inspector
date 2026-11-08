//! Rendering regression: every page of the fixtures against a baseline PNG in
//! `tests/render/`, within a tolerance.
//!
//! - `UPDATE_RENDER=1 cargo test -p pdfi-render --test regression` writes the
//!   baselines again (look at what changed with `git diff` and an image viewer
//!   before committing them).
//! - On a mismatch the rendering and a picture of the differences are written
//!   to `target/render-regression/` (`<name>.png` and `<name>.diff.png`).
//! - `PDFI_RENDER_CORPUS=<dir>` also checks every PDF under `<dir>` (the first
//!   `PDFI_RENDER_PAGES` pages, 5 by default) against baselines kept in
//!   `<dir>/.render-baseline/`. A file without baselines is skipped (run with
//!   `UPDATE_RENDER=1` once to make them).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfi_content::{DocContent, Target};
use pdfi_model::DocModel;
use pdfi_render::{AnnotPaint, DocRender, Layer, Options, Output, render};
use pdfi_syntax::{MemSource, OpenOptions, PdfFile};
use tiny_skia::{IntSize, Pixmap};

/// Pixels per point.
const SCALE: f64 = 1.0;
/// A pixel differs when a channel (premultiplied) is off by more than this:
/// room for rounding that differs between CPUs (SIMD paths of tiny-skia).
const CHANNEL_TOLERANCE: u8 = 3;
/// Slightly different pixels let through, per pixel of the rendering: the
/// coverage of antialiased edges moving a little.
const SLIGHT_SHARE: usize = 1_000;
/// A pixel differs a lot when a channel is off by more than this: a mark
/// that moved, appeared or changed color rather than an edge shading
/// differently.
const MUCH: u8 = 32;
/// Pixels that differ a lot let through. Kept below what one small glyph
/// covers (a 6×7-pixel mark must fail).
const MUCH_ALLOWED: usize = 4;

/// The fixtures that draw something worth keeping, and whether their pages
/// have annotation appearances to check as a layer of their own.
const FIXTURES: &[(&str, bool)] = &[
    ("render.pdf", false),
    ("paint.pdf", false),
    ("fonts.pdf", false),
    ("content.pdf", false),
    ("model.pdf", false),
    ("annots.pdf", true),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn updating() -> bool {
    std::env::var_os("UPDATE_RENDER").is_some_and(|v| !v.is_empty() && v != "0")
}

struct Doc {
    file: PdfFile,
    model: DocModel,
    content: DocContent,
    render: DocRender,
}

impl Doc {
    fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let file = PdfFile::open(Arc::new(MemSource::new(bytes)), OpenOptions::default())
            .map_err(|e| e.to_string())?;
        Ok(Doc {
            file,
            model: DocModel::new(),
            content: DocContent::new(),
            render: DocRender::new(),
        })
    }

    fn pages(&self) -> u64 {
        self.model.on(&self.file).page_count()
    }

    fn render(&self, page: u64, layer: Layer) -> Result<Output, String> {
        let c = self.content.on(self.model.on(&self.file));
        let a = c.analyze(Target::Page(page)).map_err(|e| e.to_string())?;
        let o = Options {
            scale: SCALE,
            tile: None,
            until: None,
            layer,
        };
        render(&self.render, &c, &a, &o, &|| false).map_err(|e| format!("{e:?}"))
    }

    /// The page's annotation appearances as `Layer::Annots`, or `None` when
    /// it has none to paint.
    fn annots(&self, page: u64) -> Option<Layer> {
        let (_, annots) = self.model.on(&self.file).annots(page, 0, 10_000).ok()?;
        let list = annots
            .iter()
            .filter_map(|a| {
                let (id, matrix) = a.paint()?;
                Some(AnnotPaint {
                    id,
                    matrix,
                    opacity: a.opacity.unwrap_or(1.0),
                })
            })
            .collect::<Vec<_>>();
        (!list.is_empty()).then_some(Layer::Annots(list))
    }
}

/// The rendering as a pixmap (premultiplied, as PNG files load).
fn pixmap(o: &Output) -> Pixmap {
    let mut data = o.rgba.clone();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(data, IntSize::from_wh(o.width, o.height).unwrap()).unwrap()
}

/// What differs between a rendering and its baseline.
struct Diff {
    /// Off by more than `CHANNEL_TOLERANCE`.
    differing: usize,
    /// Off by more than `MUCH`.
    much: usize,
    max: u8,
    /// The baseline faded, differing pixels in red.
    picture: Pixmap,
}

fn compare(got: &Pixmap, want: &Pixmap) -> Diff {
    let mut picture = want.clone();
    let mut differing = 0;
    let mut much = 0;
    let mut max = 0;
    let out = picture.data_mut();
    for (i, (g, w)) in got
        .data()
        .chunks_exact(4)
        .zip(want.data().chunks_exact(4))
        .enumerate()
    {
        let d = g.iter().zip(w).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        max = max.max(d);
        let px = &mut out[i * 4..i * 4 + 4];
        if d > CHANNEL_TOLERANCE {
            differing += 1;
            much += usize::from(d > MUCH);
            px.copy_from_slice(&[255, 0, 0, 255]);
        } else {
            // Over white, a quarter of the strength.
            let a = w[3] as u32;
            for (o, c) in px[..3].iter_mut().zip(w) {
                *o = (255 - (255 * a - *c as u32 * 255) / 255 / 4) as u8;
            }
            px[3] = 255;
        }
    }
    Diff {
        differing,
        much,
        max,
        picture,
    }
}

/// One rendering to check: where its baseline is and what it is called in
/// messages and in `target/render-regression/`.
struct Case {
    name: String,
    baseline: PathBuf,
}

/// Checks (or, when updating, writes) one rendering. `Err` is the failure.
fn check(case: &Case, got: Result<Output, String>) -> Result<(), String> {
    let got = got.map_err(|e| format!("{}: does not render: {e}", case.name))?;
    let got = pixmap(&got);
    if updating() {
        std::fs::create_dir_all(case.baseline.parent().unwrap()).unwrap();
        let png = got.encode_png().map_err(|e| e.to_string())?;
        // Leave a baseline alone while it still matches, so its bytes (and
        // git) stay quiet.
        let same = Pixmap::load_png(&case.baseline)
            .is_ok_and(|want| want.data() == got.data() && want.width() == got.width());
        if !same {
            std::fs::write(&case.baseline, png).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let want = Pixmap::load_png(&case.baseline).map_err(|e| {
        format!(
            "{}: no baseline at {} ({e}); run with UPDATE_RENDER=1",
            case.name,
            case.baseline.display()
        )
    })?;
    let out = root().join("target/render-regression");
    let keep = |diff: Option<&Pixmap>| {
        let base = out.join(&case.name);
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        got.save_png(base.with_extension("png")).unwrap();
        if let Some(d) = diff {
            d.save_png(base.with_extension("diff.png")).unwrap();
        }
        base.with_extension("png")
    };
    if (got.width(), got.height()) != (want.width(), want.height()) {
        let at = keep(None);
        return Err(format!(
            "{}: {}×{} pixels, the baseline has {}×{} (rendering in {})",
            case.name,
            got.width(),
            got.height(),
            want.width(),
            want.height(),
            at.display()
        ));
    }
    let d = compare(&got, &want);
    let slight = (got.width() as usize * got.height() as usize / SLIGHT_SHARE).max(MUCH_ALLOWED);
    if d.differing > slight || d.much > MUCH_ALLOWED {
        let at = keep(Some(&d.picture));
        return Err(format!(
            "{}: {} pixels differ (at most {slight}), {} by more than {MUCH} (at most \
             {MUCH_ALLOWED}), by up to {} (rendering and its diff in {}{{.png,.diff.png}})",
            case.name,
            d.differing,
            d.much,
            d.max,
            at.with_extension("").display()
        ));
    }
    Ok(())
}

/// Reports every failure at once rather than stopping at the first.
#[track_caller]
fn finish(failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} renderings do not match their baselines:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn fixtures_match_their_baselines() {
    let dir = root().join("tests/render");
    let mut failures = Vec::new();
    let mut seen = Vec::new();
    for &(file, annots) in FIXTURES {
        let doc = Doc::open(&root().join("tests/fixtures").join(file)).unwrap();
        let stem = file.trim_end_matches(".pdf");
        let pages = doc.pages();
        assert!(pages > 0, "{file} has no pages");
        for page in 0..pages {
            let name = format!("{stem}-p{}", page + 1);
            let case = Case {
                baseline: dir.join(format!("{name}.png")),
                name,
            };
            seen.push(case.baseline.clone());
            failures.extend(check(&case, doc.render(page, Layer::Content)).err());
            if !annots {
                continue;
            }
            let Some(layer) = doc.annots(page) else {
                failures.push(format!("{stem} page {}: no annotations to paint", page + 1));
                continue;
            };
            let name = format!("{stem}-p{}-annots", page + 1);
            let case = Case {
                baseline: dir.join(format!("{name}.png")),
                name,
            };
            seen.push(case.baseline.clone());
            failures.extend(check(&case, doc.render(page, layer)).err());
        }
    }
    // Baselines of pages that are gone.
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "png") && !seen.contains(&p) {
            if updating() {
                std::fs::remove_file(&p).unwrap();
            } else {
                failures.push(format!("{}: no such page any more", p.display()));
            }
        }
    }
    finish(failures);
}

/// PDFs under `dir`, sorted, skipping hidden directories (the baselines).
fn pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.sort();
    for p in entries {
        let hidden = p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if hidden {
            continue;
        }
        if p.is_dir() {
            pdfs(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            out.push(p);
        }
    }
}

#[test]
fn corpus_matches_its_baselines() {
    let Some(dir) = std::env::var_os("PDFI_RENDER_CORPUS").map(PathBuf::from) else {
        return;
    };
    let limit: u64 = std::env::var("PDFI_RENDER_PAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let baselines = dir.join(".render-baseline");
    let mut files = Vec::new();
    pdfs(&dir, &mut files);
    assert!(!files.is_empty(), "no PDFs under {}", dir.display());

    let check_file = |path: &Path| -> Vec<String> {
        let rel = path.strip_prefix(&dir).unwrap().with_extension("");
        let rel = rel.to_string_lossy();
        let base = baselines.join(&*rel);
        let has_baselines = base.is_dir();
        if !updating() && !has_baselines {
            return Vec::new();
        }
        let doc = match Doc::open(path) {
            Ok(d) => d,
            // Files that do not open are left to the other tests.
            Err(_) if !has_baselines => return Vec::new(),
            Err(e) => return vec![format!("{rel}: does not open: {e}")],
        };
        let mut failures = Vec::new();
        for page in 0..doc.pages().min(limit) {
            let case = Case {
                name: format!("corpus/{rel}/p{}", page + 1),
                baseline: base.join(format!("p{}.png", page + 1)),
            };
            if updating() || case.baseline.exists() {
                // Pages that did not render when the baselines were made
                // have none, and stay unchecked.
                let got = doc.render(page, Layer::Content);
                if updating() && got.is_err() {
                    continue;
                }
                failures.extend(check(&case, got).err());
            }
        }
        failures
    };

    // A thread per core, each taking the next file.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut failures = std::thread::scope(|s| {
        let workers = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(f) = files.get(i) else { break out };
                        out.extend(check_file(f));
                    }
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect::<Vec<_>>()
    });
    failures.sort();
    finish(failures);
}
