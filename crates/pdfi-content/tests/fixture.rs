use std::sync::Arc;

use pdfi_content::extract::{Extract, Region, Select};
use pdfi_content::interp::{ElementKind, Kind};
use pdfi_content::op::Operator;
use pdfi_content::{Analysis, DocContent, Target};
use pdfi_model::{DocModel, Rect};
use pdfi_syntax::{DictExt, MemSource, ObjId, Object, OpenOptions, PdfFile, Severity};

fn open_bytes(data: Vec<u8>) -> PdfFile {
    PdfFile::open(Arc::new(MemSource::new(data)), OpenOptions::default()).unwrap()
}

fn open(name: &str) -> PdfFile {
    let p = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    open_bytes(std::fs::read(&p).unwrap())
}

/// A one-page document with `content` and a Helvetica `/F1`.
fn one_page(content: &[u8]) -> PdfFile {
    let objs: [Vec<u8>; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>".to_vec(),
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
    open_bytes(out)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect { x0, y0, x1, y1 }
}

fn codes(a: &Analysis, op: u32) -> Vec<&str> {
    a.issues_of(op).iter().map(|i| i.code).collect()
}

fn find(a: &Analysis, from: usize, op: Operator) -> u32 {
    (from
        + a.read_ops(from, a.ops)
            .iter()
            .position(|o| o.operator == op)
            .unwrap()) as u32
}

#[test]
fn valid_page() {
    let f = open("content.pdf");
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(&f));
    let a = c.analyze(Target::Page(0)).unwrap();
    assert_eq!(a.kind, Kind::Page);
    assert_eq!(a.bounds, Some(rect(0.0, 0.0, 612.0, 792.0)));
    assert_eq!(a.ops, 56);
    assert!(
        a.issues.iter().all(|i| i.severity == Severity::Info),
        "{:?}",
        a.issues
    );

    let fill = a.element_of(4).unwrap();
    assert_eq!(
        (fill.kind, fill.bbox),
        (ElementKind::Fill, rect(50.0, 50.0, 250.0, 150.0))
    );
    // Stroked with line width 2: grows by 1.
    assert_eq!(
        a.element_of(9).unwrap().bbox,
        rect(59.0, 59.0, 241.0, 141.0)
    );
    assert_eq!(a.element_of(14).unwrap().kind, ElementKind::Clip);

    let tj = find(&a, 0, Operator::ShowText);
    assert_eq!(a.text_of(tj), Some("Hello"));
    // Helvetica: H e l l o = 2278 units at 24 pt.
    let e = a.element_of(tj).unwrap();
    assert!(
        (e.bbox.x1 - (72.0 + 2.278 * 24.0)).abs() < 1e-9,
        "{:?}",
        e.bbox
    );
    assert!(!e.hidden);
    assert_eq!(a.text_of(tj + 1), Some("W orld"));
    // The Type 0 font with /ToUnicode, invisible (3 Tr).
    let tj2 = find(&a, tj as usize + 2, Operator::ShowText);
    assert_eq!(a.text_of(tj2), Some("AB"));
    let e = a.element_of(tj2).unwrap();
    assert!(e.hidden);
    // /W gives A 500, B 600 at 12 pt.
    assert!(
        (e.bbox.x1 - (72.0 + 1.1 * 12.0)).abs() < 1e-9,
        "{:?}",
        e.bbox
    );

    let kinds: Vec<_> = a.elements.iter().map(|e| e.kind).collect();
    for k in [
        ElementKind::Image,
        ElementKind::Form,
        ElementKind::InlineImage,
        ElementKind::Shading,
    ] {
        assert!(kinds.contains(&k), "{k:?} in {kinds:?}");
    }
    let form = a
        .elements
        .iter()
        .find(|e| e.kind == ElementKind::Form)
        .unwrap();
    // The form's stroke and text, scaled by its /Matrix, clipped to its /BBox.
    assert_eq!(form.bbox, rect(100.0, 100.0, 201.0, 201.0));
    let unknown = find(&a, 0, Operator::Unknown(b"foo".as_slice().into()));
    assert_eq!(codes(&a, unknown), ["UnknownOperator"]);
    // Nesting: q … Q at 0, inside at 1, inside BT at 2.
    assert_eq!((a.depth[0], a.depth[1], a.depth[10]), (0, 1, 0));
    assert_eq!(a.depth[tj as usize], 2);

    let (before, after) = c.state_at(&a, tj as usize + 1).unwrap();
    let g = &before.g;
    assert_eq!(g.text.font.as_ref().unwrap().name, "F1");
    assert_eq!(g.text.size, 24.0);
    assert_eq!(g.ext_gstates, ["GS1"]);
    assert_eq!(
        (g.line_width, g.fill_alpha, g.blend.as_str()),
        (3.0, 0.5, "Multiply")
    );
    assert_eq!(g.fill.family, "ICCBased");
    assert_eq!(g.fill.components, [0.1, 0.2, 0.3]);
    assert_eq!(g.clip, Some(rect(0.0, 0.0, 612.0, 720.0)));
    assert_eq!(before.tm[4], e_end(&a, tj));
    assert!(after.tm[4] > before.tm[4]);
    assert_eq!(before.stack.len(), 1);
    assert!(before.in_text);
}

fn e_end(a: &Analysis, op: u32) -> f64 {
    a.element_of(op).unwrap().bbox.x1
}

#[test]
fn problems() {
    let f = open("content.pdf");
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(&f));
    let a = c.analyze(Target::Page(1)).unwrap();
    assert_eq!(codes(&a, 0), ["Unbalanced"]);
    assert_eq!(codes(&a, 1), ["Placement", "NoFont"]);
    assert_eq!(codes(&a, 3), ["UndefinedResource"]);
    assert_eq!(codes(&a, 5), ["Placement"]);
    assert_eq!(codes(&a, 10), ["Operands"]);
    assert_eq!(codes(&a, 14), ["Placement", "Placement"]);
    assert_eq!(codes(&a, 16), ["Operands"]);
    assert_eq!(codes(&a, 18), ["Operands"]);
    assert_eq!(codes(&a, 21), ["ColorSpace"]);
    assert_eq!(codes(&a, 22), ["UndefinedResource"]);
    assert_eq!(codes(&a, 28), ["Unbalanced"]);
    assert_eq!(codes(&a, 29), ["UnknownOperator"]);
    assert_eq!(codes(&a, 30), ["NoPath"]);
    let whole: Vec<_> = a
        .issues
        .iter()
        .filter(|i| i.op.is_none())
        .map(|i| &i.message)
        .collect();
    assert_eq!(
        whole,
        ["2 q without Q at the end", "BT without ET at the end"]
    );
    // The extra operand of cm is skipped: the last six are used.
    let (_, after) = c.state_at(&a, 18).unwrap();
    assert_eq!(after.g.ctm, [2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
}

#[test]
fn split_contents_and_streams() {
    let f = open("content.pdf");
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(&f));
    let a = c.analyze(Target::Page(2)).unwrap();
    assert_eq!(a.sources.len(), 2);
    let rg = &a.read_ops(0, 1)[0];
    assert_eq!(rg.operands.len(), 3);
    assert_eq!(rg.origin.unwrap().source, 1);
    assert!(a.issues.is_empty(), "{:?}", a.issues);

    let form = c.analyze(Target::Stream(ObjId::new(31, 0))).unwrap();
    assert_eq!(form.kind, Kind::Form);
    assert_eq!(form.bounds, Some(rect(0.0, 0.0, 100.0, 100.0)));
    assert_eq!(form.text_of(3), Some("in form"));
    assert!(c.analyze(Target::Stream(ObjId::new(20, 0))).is_err());
    // Cached.
    assert!(Arc::ptr_eq(&a, &c.analyze(Target::Page(2)).unwrap()));
}

#[test]
fn state_past_checkpoints() {
    let mut content = Vec::new();
    for i in 0..3000 {
        content.extend_from_slice(format!("{i} w q 1 0 0 1 1 0 cm Q\n").as_bytes());
    }
    content.extend_from_slice(b"BT /F1 10 Tf (x) Tj ET");
    let f = one_page(&content);
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(&f));
    let a = c.analyze(Target::Page(0)).unwrap();
    assert_eq!(a.ops, 3000 * 4 + 4);
    // Read again from the middle of a checkpoint interval.
    let ops = a.read_ops(4 * 2500 + 2, 2);
    assert_eq!(ops[0].operator, Operator::Concat);
    assert_eq!(ops[1].operator, Operator::Restore);
    assert!(a.issues.is_empty(), "{:?}", a.issues);
    // Op 4k + 2 is the cm of round k, inside q.
    let (before, after) = c.state_at(&a, 4 * 2500 + 2).unwrap();
    assert_eq!(before.g.line_width, 2500.0);
    assert_eq!(before.stack.len(), 1);
    assert_eq!(after.g.ctm[4], 1.0);
    let (_, after) = c.state_at(&a, 4 * 2500 + 3).unwrap();
    assert_eq!(after.g.ctm[4], 0.0);
    assert_eq!(a.depth[4 * 2500 + 2], 1);
    assert!(c.state_at(&a, a.ops).is_none());
}

#[test]
fn text_clipping_modes() {
    // fonts.pdf: `BT 7 Tr … (A) Tj ET` then a fill: the clip after ET is
    // the glyph's box (advance 70 × ascent / descent 80 / -20 at 100 pt).
    let file = open("fonts.pdf");
    let (model, content) = (DocModel::new(), DocContent::new());
    let c = content.on(model.on(&file));
    let a = c.analyze(Target::Page(0)).unwrap();
    let ops = a.read_ops(0, a.ops);
    let tr7 = ops
        .iter()
        .position(|o| o.operator == Operator::RenderMode && o.operands[0].obj.as_i64() == Some(7))
        .unwrap();
    let fill = tr7
        + ops[tr7..]
            .iter()
            .position(|o| o.operator == Operator::Fill)
            .unwrap();
    let (before, _) = c.state_at(&a, fill).unwrap();
    assert_eq!(
        before.g.clip,
        Some(Rect {
            x0: 170.0,
            y0: 330.0,
            x1: 240.0,
            y1: 430.0
        })
    );
    // The text itself is hidden (Tr 7), the fill is not.
    assert!(a.element_of(tr7 as u32 + 3).is_some_and(|e| e.hidden));
    assert!(a.element_of(fill as u32).is_some_and(|e| !e.hidden));
    // Text after the next BT is not clipped by it.
    let (s, _) = c.state_at(&a, fill + 3).unwrap();
    assert_eq!(
        s.g.clip,
        Some(Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0
        })
    );
}

/// A document of `objs` (object 1 the catalog).
fn document(objs: &[&[u8]]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let n = objs.len() + 1;
    out.extend_from_slice(format!("xref\n0 {n}\n0000000000 65535 f\r\n").as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    [
        format!("<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes(),
        data,
        b"\nendstream",
    ]
    .concat()
}

/// Cut `rect` out of page 0; the file written and its page's content.
fn cut(f: &PdfFile, rect: Rect, select: Select) -> (Extract, PdfFile, String) {
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(f));
    let a = c.analyze(Target::Page(0)).unwrap();
    let region = Region {
        rect,
        select,
        fit: false,
    };
    let x = c.extract(&a, &region, &|| false).unwrap();
    let out = open_bytes(x.pdf.clone());
    assert!(
        out.diagnostics()
            .iter()
            .all(|d| d.severity == Severity::Info),
        "{:?}",
        out.diagnostics()
    );
    let page = out.get(ObjId::new(3, 0)).unwrap();
    let id = match page.node.obj.as_dict().unwrap().get_key("Contents") {
        Some(Object::Ref(id)) => *id,
        o => panic!("{o:?}"),
    };
    let text = stream_text(&out, id);
    (x, out, text)
}

fn stream_text(f: &PdfFile, id: ObjId) -> String {
    let h = f.get(id).unwrap();
    let s = h.node.obj.as_stream().unwrap();
    String::from_utf8(f.decode_stream(id, s, None).data).unwrap()
}

#[test]
fn extract_paths_and_text() {
    let f = one_page(
        b"q 1 0 0 RG 2 w 10 10 m 90 90 l S Q\n\
          0 0 1 rg 300 300 50 50 re f\n\
          10 10 500 500 re W n\n\
          BT /F1 12 Tf 20 200 Td (Left) Tj (Right) Tj 0 -20 TD (Next) Tj ET",
    );
    // "Left" spans 20–40, "Right" 40–73, "Next" 20–44 on the line below.
    let (x, out, text) = cut(&f, rect(0.0, 0.0, 200.0, 250.0), Select::Inside);
    assert_eq!((x.kept, x.dropped), (4, 1));
    assert!(text.contains("10 10 m\n90 90 l\nS\n"), "{text}");
    assert!(!text.contains("300 300"), "{text}");
    // The clip stays.
    assert!(text.contains("10 10 500 500 re\nW\nn\n"), "{text}");
    let page = out.get(ObjId::new(3, 0)).unwrap();
    let mb = page
        .node
        .obj
        .as_dict()
        .unwrap()
        .get_key("MediaBox")
        .unwrap();
    assert_eq!(Rect::read(&out, mb), Some(rect(0.0, 0.0, 200.0, 250.0)));

    // Only "Right": "Left" is shown invisibly for where it ends; the line
    // after goes, but not the leading `TD` sets.
    let (x, _, text) = cut(&f, rect(39.0, 190.0, 200.0, 215.0), Select::Inside);
    assert_eq!((x.kept, x.dropped), (1, 4));
    assert!(
        text.contains("20 200 Td\n3 Tr\n(Left) Tj\n0 Tr\n(Right) Tj"),
        "{text}"
    );
    assert!(!text.contains("Next") && text.contains("20.0 TL"), "{text}");
    // The font is kept, the stroke and fill are not.
    assert!(!text.contains("\nS\n") && !text.contains("\nf\n"), "{text}");

    // Overlapping: the line below too, no invisible text needed.
    let (x, _, text) = cut(&f, rect(21.0, 170.0, 30.0, 215.0), Select::Overlaps);
    assert_eq!(x.kept, 2);
    assert!(text.contains("(Left) Tj\n0 -20 TD\n(Next) Tj"), "{text}");
    assert!(!text.contains("Right") && !text.contains("Tr"), "{text}");
}

#[test]
fn extract_cuts_forms() {
    let page = stream("", b"q 1 0 0 1 100 0 cm /Fm1 Do Q /Fm1 Do");
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 600 800] /Resources << /ExtGState << /G0 << /CA 0.5 >> /G1 << /CA 1 >> >> >>",
        b"/G0 gs 0 0 1 rg 10 10 50 50 re f 1 0 0 rg 300 300 50 50 re f",
    );
    let f = open_bytes(document(&[
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>",
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Fm1 5 0 R >> >> >>",
        &page,
        &form,
    ]));
    // The first square of the first form (at 110–160): the form is cut,
    // the second `Do` (its square at 10–60) not taken.
    let (x, out, text) = cut(&f, rect(100.0, 0.0, 200.0, 200.0), Select::Inside);
    assert_eq!((x.kept, x.dropped, x.forms), (1, 3, 1));
    assert_eq!(text, "q\n1 0 0 1 100 0 cm\n/PdfiCut0 Do\nQ\n");
    let page = out.get(ObjId::new(3, 0)).unwrap();
    let res = page
        .node
        .obj
        .as_dict()
        .unwrap()
        .get_key("Resources")
        .unwrap();
    let xo = res
        .as_dict()
        .unwrap()
        .get_key("XObject")
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(xo.len(), 1);
    let Some(Object::Ref(id)) = xo.get_key("PdfiCut0") else {
        panic!()
    };
    let inner = stream_text(&out, *id);
    assert_eq!(inner, "/G0 gs\n0 0 1 rg\n10 10 50 50 re\nf\n1 0 0 rg\n");
    let form = out.get(*id).unwrap();
    let d = &form.node.obj.as_stream().unwrap().dict;
    assert!(d.get_key("BBox").is_some());
    // Only the graphics state used.
    let gs = d
        .get_key("Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get_key("ExtGState");
    assert_eq!(gs.unwrap().as_dict().unwrap().len(), 1);

    // Both forms wholly inside: painted as they are.
    let (x, _, text) = cut(&f, rect(0.0, 0.0, 600.0, 800.0), Select::Inside);
    assert_eq!((x.kept, x.dropped, x.forms), (2, 0, 0));
    assert_eq!(text, "q\n1 0 0 1 100 0 cm\n/Fm1 Do\nQ\n/Fm1 Do\n");
}
