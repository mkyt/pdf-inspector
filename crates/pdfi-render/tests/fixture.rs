use std::sync::Arc;

use pdfi_content::{DocContent, Target};
use pdfi_model::DocModel;
use pdfi_render::{DocRender, Layer, Options, Output, Tile, hit_test, render, view};
use pdfi_syntax::{MemSource, OpenOptions, PdfFile};

fn open(name: &str) -> PdfFile {
    let p = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    PdfFile::open(
        Arc::new(MemSource::new(std::fs::read(&p).unwrap())),
        OpenOptions::default(),
    )
    .unwrap()
}

struct Doc {
    file: PdfFile,
    model: DocModel,
    content: DocContent,
    render: DocRender,
}

impl Doc {
    fn new(name: &str) -> Self {
        Doc {
            file: open(name),
            model: DocModel::new(),
            content: DocContent::new(),
            render: DocRender::new(),
        }
    }

    fn render(&self, t: Target, o: &Options) -> Output {
        let c = self.content.on(self.model.on(&self.file));
        let a = c.analyze(t).unwrap();
        render(&self.render, &c, &a, o, &|| false).unwrap()
    }

    fn hits(&self, t: Target, x: f64, y: f64) -> Vec<u32> {
        let c = self.content.on(self.model.on(&self.file));
        let a = c.analyze(t).unwrap();
        hit_test(&self.render, &c, &a, (x, y), 1.0, None, &|| false)
    }
}

fn opts(scale: f64) -> Options {
    Options {
        scale,
        tile: None,
        until: None,
        layer: Layer::Content,
    }
}

/// RGBA at a pixel.
fn px(o: &Output, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * o.width + x) * 4) as usize;
    o.rgba[i..i + 4].try_into().unwrap()
}

/// Whether two colors are within `tol` per channel.
fn near(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

#[track_caller]
fn assert_px(o: &Output, x: u32, y: u32, want: [u8; 4]) {
    let got = px(o, x, y);
    assert!(
        near(got, want, 3),
        "pixel ({x}, {y}) is {got:?}, want {want:?}"
    );
}

const WHITE: [u8; 4] = [255; 4];

#[test]
fn paths_clips_and_colors() {
    let d = Doc::new("render.pdf");
    let o = d.render(Target::Page(0), &opts(1.0));
    assert_eq!((o.width, o.height), (400, 400));
    assert!(o.notes.is_empty(), "{:?}", o.notes);
    // View y = 400 - page y.
    // Even-odd square with a hole.
    assert_px(&o, 30, 370, [0, 0, 255, 255]);
    assert_px(&o, 100, 300, WHITE);
    // Circle clip: center red, the clipped rectangle's corner white.
    assert_px(&o, 300, 300, [255, 0, 0, 255]);
    assert_px(&o, 210, 390, WHITE);
    // Nested clips: only 20..50 × 200..230 is green.
    assert_px(&o, 35, 185, [0, 255, 0, 255]);
    assert_px(&o, 60, 185, WHITE);
    assert_px(&o, 35, 165, WHITE);
    // CMYK yellow, Indexed blue, Separation through its tint transform
    // (cyan), half-transparent blue over white.
    assert_px(&o, 230, 70, [255, 255, 0, 255]);
    assert_px(&o, 300, 70, [0, 0, 255, 255]);
    assert_px(&o, 365, 80, [0, 255, 255, 255]);
    assert_px(&o, 370, 190, [128, 128, 255, 255]);
    // Dashes: green, with gaps along the line.
    assert_px(&o, 26, 150, [0, 153, 0, 255]);
    // (Round caps close the gaps at the center; they show at the edge.)
    let row: Vec<[u8; 4]> = (20..80).map(|x| px(&o, x, 148)).collect();
    assert!(row.iter().any(|p| p[0] > 80), "no gap: {row:?}");
    // Lab: a pink.
    let lab = px(&o, 365, 30);
    assert!(lab[0] > 200 && lab[1] < 80 && lab[2] > 80, "{lab:?}");
}

#[test]
fn images() {
    let d = Doc::new("render.pdf");
    let o = d.render(Target::Page(1), &opts(1.0));
    assert!(o.notes.is_empty(), "{:?}", o.notes);
    // 1-bit checker: first sample is 1 (white), the next 0 (black).
    assert_px(&o, 25, 25, WHITE);
    assert_px(&o, 35, 25, [0, 0, 0, 255]);
    // 4-bit Indexed: red, green, blue, yellow.
    assert_px(&o, 130, 30, [255, 0, 0, 255]);
    assert_px(&o, 150, 30, [0, 255, 0, 255]);
    assert_px(&o, 190, 30, [255, 255, 0, 255]);
    // 16-bit RGB.
    assert_px(&o, 240, 40, [255, 0, 0, 255]);
    assert_px(&o, 280, 80, [128, 128, 128, 255]);
    // Stencil: sample 0 paints the fill color, 1 leaves the backdrop.
    assert_px(&o, 315, 25, WHITE);
    assert_px(&o, 345, 25, [255, 0, 0, 255]);
    // Soft mask over blue: red opaque, green at 50 %, nothing at 0 %.
    assert_px(&o, 40, 140, [255, 0, 0, 255]);
    assert_px(&o, 80, 140, [0, 128, 127, 255]);
    assert_px(&o, 80, 180, [0, 0, 255, 255]);
    // Color key: black samples show the blue backdrop.
    assert_px(&o, 140, 140, [255, 0, 0, 255]);
    assert_px(&o, 180, 140, [0, 0, 255, 255]);
    // JPEG: gradient corners, gray, CMYK through /Decode (red).
    let j = px(&o, 225, 125);
    assert!(j[2] > 200 && j[0] < 60, "{j:?}");
    let g = px(&o, 30, 285);
    assert!(g[0] == g[1] && g[1] == g[2], "{g:?}");
    assert_px(&o, 260, 260, [255, 0, 0, 255]);
    // Inline image with /D [1 0]: 0 → white, 255 → black.
    assert_px(&o, 315, 260, WHITE);
    assert_px(&o, 385, 260, [0, 0, 0, 255]);
}

#[test]
fn rotated_page_and_form_clip() {
    let d = Doc::new("render.pdf");
    let c = d.content.on(d.model.on(&d.file));
    let a = c.analyze(Target::Page(2)).unwrap();
    let v = view(&a);
    assert_eq!((v.width, v.height), (200.0, 300.0));
    let o = d.render(Target::Page(2), &opts(1.0));
    // Content (x, y) shows at (y, x) on a page turned 90° clockwise.
    assert_px(&o, 100, 200, [204, 51, 51, 255]);
    // The form paints a large rectangle; its /BBox clips it.
    assert_px(&o, 30, 30, WHITE);
    assert_px(&o, 160, 100, WHITE);
}

#[test]
fn tiles_match_the_whole() {
    // Curves are flattened relative to the bitmap's edges, so anti-aliased
    // pixels along them may differ slightly between tiles; seams may not.
    let d = Doc::new("render.pdf");
    for page in 0..3 {
        let whole = d.render(Target::Page(page), &opts(0.75));
        let (w, h) = (whole.width, whole.height);
        let ts = 64;
        for ty in (0..h).step_by(ts) {
            for tx in (0..w).step_by(ts) {
                let mut o = opts(0.75);
                o.tile = Some(Tile {
                    x: tx,
                    y: ty,
                    width: ts as u32,
                    height: ts as u32,
                });
                let t = d.render(Target::Page(page), &o);
                assert_eq!(
                    (t.width, t.height),
                    ((w - tx).min(ts as u32), (h - ty).min(ts as u32))
                );
                for y in 0..t.height {
                    for x in 0..t.width {
                        let (a, b) = (px(&t, x, y), px(&whole, tx + x, ty + y));
                        assert!(
                            near(a, b, 48),
                            "page {page}: tile ({tx}, {ty}) differs at ({x}, {y}): {a:?} vs {b:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn step_and_highlight() {
    let d = Doc::new("render.pdf");
    // Up to the even-odd fill (op 4: `q rg re re f*`): only it is painted.
    let mut o = opts(1.0);
    o.until = Some(4);
    let r = d.render(Target::Page(0), &o);
    assert_eq!(r.ops, 5);
    assert_px(&r, 30, 370, [0, 0, 255, 255]);
    assert_px(&r, 300, 300, WHITE);
    // A highlight layer: transparent except the chosen operation.
    let mut o = opts(1.0);
    o.layer = Layer::Highlight(vec![4]);
    let r = d.render(Target::Page(0), &o);
    assert_eq!(px(&r, 30, 370)[3], 255);
    assert_eq!(px(&r, 100, 300)[3], 0);
    assert_eq!(px(&r, 300, 300)[3], 0);
}

#[test]
fn hits() {
    let d = Doc::new("render.pdf");
    // The even-odd fill is op 4; its hole is empty.
    assert_eq!(d.hits(Target::Page(0), 30.0, 370.0), vec![4]);
    assert!(d.hits(Target::Page(0), 100.0, 300.0).is_empty());
    // Inside the circle clip, only the clipped fill.
    let circle = d.hits(Target::Page(0), 300.0, 300.0);
    assert_eq!(circle.len(), 1);
    // The translucent square over the circle: topmost first.
    let both = d.hits(Target::Page(0), 310.0, 230.0);
    assert_eq!(both.len(), 2);
    assert_eq!(both[1], circle[0]);
    // The form XObject counts as its `Do`.
    let f = d.hits(Target::Page(2), 100.0, 200.0);
    assert_eq!(f.len(), 1);
    let c = d.content.on(d.model.on(&d.file));
    let a = c.analyze(Target::Page(2)).unwrap();
    let op = &a.read_ops(f[0] as usize, 1)[0];
    assert_eq!(op.operator.name(), "Do");
}

#[test]
fn notes_for_what_is_left_out() {
    let d = Doc::new("content.pdf");
    let o = d.render(Target::Page(0), &opts(0.5));
    let notes: Vec<&str> = o.notes.iter().map(|(n, _)| n.as_str()).collect();
    assert!(notes.iter().any(|n| n.contains("shadings")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("glyph boxes")), "{notes:?}");
}

#[test]
fn bad_requests() {
    let d = Doc::new("render.pdf");
    let c = d.content.on(d.model.on(&d.file));
    let a = c.analyze(Target::Page(0)).unwrap();
    let mut o = opts(0.0);
    assert!(render(&d.render, &c, &a, &o, &|| false).is_err());
    o.scale = 100.0;
    assert!(matches!(
        render(&d.render, &c, &a, &o, &|| false),
        Err(pdfi_render::Error::TooLarge(_))
    ));
    o.scale = 1.0;
    o.tile = Some(Tile {
        x: 500,
        y: 0,
        width: 10,
        height: 10,
    });
    assert!(render(&d.render, &c, &a, &o, &|| false).is_err());
    // Cancelled before the first operation.
    o.tile = None;
    let r = render(&d.render, &c, &a, &o, &|| true).unwrap();
    assert!(r.cancelled);
    assert_eq!(r.ops, 0);
}

/// A one-page document with `content`.
fn one_page(content: &[u8]) -> Doc {
    let objs: [Vec<u8>; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
            content,
            b"\nendstream",
        ]
        .concat(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f\r\n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    Doc {
        file: PdfFile::open(Arc::new(MemSource::new(out)), OpenOptions::default()).unwrap(),
        model: DocModel::new(),
        content: DocContent::new(),
        render: DocRender::new(),
    }
}

#[test]
fn hostile_contents_finish() {
    let cases: [&[u8]; 7] = [
        b"1e300 0 0 1e300 0 0 cm 0 0 1 1 re f 0 0 m 1 1 l S",
        b"0 0 0 0 0 0 cm 0 0 1 1 re f 5 w 0 0 m 1 1 l S",
        b"1e-30 0 0 1e-30 0 0 cm 0 0 1e40 1e40 re f",
        b"[0.0001] 0 d 1 w 0 0 m 1000000 0 l S",
        b"q 1e30 1e30 m 1e-30 1e-30 l W n 0 0 200 200 re f Q",
        b"BI /W 1000000 /H 1000000 /CS /G /BPC 8 ID \x00\x00 EI",
        b"0 0 m 100 100 l 200 0 l h W* n 0 0 200 200 re f",
    ];
    for c in cases {
        let d = one_page(c);
        let o = d.render(Target::Page(0), &opts(1.0));
        assert_eq!((o.width, o.height), (200, 200));
    }
    // Many clips, each followed by a fill: masks build one path at a time.
    let mut c = Vec::new();
    for i in 0..5_000 {
        c.extend_from_slice(
            format!("0 0 {} 200 re W n 0 0 1 rg 0 0 1 1 re f\n", 200 - i % 100).as_bytes(),
        );
    }
    let t = std::time::Instant::now();
    let o = one_page(&c).render(Target::Page(0), &opts(1.0));
    assert_px(&o, 0, 199, [0, 0, 255, 255]);
    assert!(t.elapsed().as_secs() < 20, "{:?}", t.elapsed());
}
