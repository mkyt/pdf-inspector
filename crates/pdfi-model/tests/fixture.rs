use std::sync::Arc;

use pdfi_model::dest::Link;
use pdfi_model::inventory::{Builder, TRAILER, ref_paths};
use pdfi_model::pages::Origin;
use pdfi_model::{DocModel, Rect};
use pdfi_syntax::{MemSource, ObjId, OpenOptions, PdfFile};

fn open(name: &str) -> PdfFile {
    let p = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read(&p).unwrap();
    PdfFile::open(Arc::new(MemSource::new(data)), OpenOptions::default()).unwrap()
}

const fn id(n: u32) -> ObjId {
    ObjId::new(n, 0)
}

#[test]
fn pages_by_index_with_inheritance() {
    let f = open("model.pdf");
    let dm = DocModel::new();
    let m = dm.on(&f);
    assert_eq!(m.page_count(), 5);
    let ids: Vec<_> = (0..5).map(|i| m.page(i).unwrap().id).collect();
    assert_eq!(ids, [id(10), id(11), id(12), id(13), id(14)]);
    assert!(m.page(5).is_err());
    for (i, &p) in ids.iter().enumerate() {
        assert_eq!(m.page_index(p), Some(i as u64));
    }
    assert_eq!(m.page_index(id(20)), None);

    let p = m.page_info(1).unwrap();
    assert_eq!(p.page.ancestors, [id(2), id(3)]);
    assert_eq!(p.boxes[0].origin, Origin::Inherited(id(2)));
    assert_eq!(p.boxes[1].origin, Origin::Inherited(id(3)));
    assert_eq!(p.boxes[2].origin, Origin::Default("CropBox"));
    assert_eq!((p.rotate, p.rotate_origin), (90, Origin::Inherited(id(3))));
    assert_eq!(p.resources_origin, Origin::Inherited(id(2)));
    assert_eq!(p.contents[0].filters, ["FlateDecode"]);
    assert_eq!(p.label.as_deref(), Some("ii"));
    assert_eq!(
        p.visible(),
        Some(Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 602.0,
            y1: 782.0
        })
    );

    let p = m.page_info(2).unwrap();
    assert_eq!(p.boxes[0].rect.unwrap().width(), 300.0);
    assert_eq!(
        p.boxes[3].rect.unwrap(),
        Rect {
            x0: 10.0,
            y0: 10.0,
            x1: 290.0,
            y1: 290.0
        }
    );
    assert_eq!(p.user_unit, 2.0);
    assert_eq!(p.label.as_deref(), Some("A-1"));

    let p = m.page_info(3).unwrap();
    assert_eq!(p.contents.len(), 2);
    assert_eq!(p.rotate, 0);
    assert!(
        p.issues.iter().any(|i| i.code == "Rotate"),
        "{:?}",
        p.issues
    );
    // Node 4 has no /Count: counted from its kids, and reported.
    assert!(
        m.issues().iter().any(|i| i.obj == Some(id(4))),
        "{:?}",
        m.issues()
    );
}

#[test]
fn outline_dests_and_annots() {
    let f = open("model.pdf");
    let dm = DocModel::new();
    let m = dm.on(&f);
    assert_eq!(*m.outline_children(None), [id(31), id(33)]);
    assert_eq!(*m.outline_children(Some(id(31))), [id(32)]);
    let ch1 = m.outline_item(id(31)).unwrap();
    assert!(ch1.has_children);
    let Some(Link::Dest(d)) = &ch1.link else {
        panic!("{ch1:?}")
    };
    assert_eq!((d.named.as_deref(), d.page_index), (Some("chap1"), Some(0)));
    let s = m.outline_item(id(32)).unwrap();
    assert_eq!(s.title, "Section 1.1");
    let Some(Link::Dest(d)) = &s.link else {
        panic!()
    };
    assert_eq!(
        (d.page_index, d.fit.as_deref(), &d.params[..]),
        (Some(1), Some("FitH"), &[Some(700.0)][..])
    );
    assert_eq!(
        m.outline_item(id(33)).unwrap().link,
        Some(Link::Uri("https://example.com/".into()))
    );

    let (n, a) = m.annots(0, 0, 100).unwrap();
    assert_eq!(n, 4);
    assert_eq!(a[0].id, None);
    let Some(Link::Dest(d)) = &a[0].link else {
        panic!()
    };
    assert_eq!(d.page_index, Some(2));
    assert_eq!(a[2].contents.as_deref(), Some("Note"));
    assert_eq!(a[3].parent, Some(id(71)));
    assert_eq!(m.annots(0, 3, 10).unwrap().1.len(), 1);
}

#[test]
fn info_and_xmp() {
    let f = open("model.pdf");
    let dm = DocModel::new();
    let m = dm.on(&f);
    let o = m.overview();
    assert_eq!(o.page_count, 5);
    assert_eq!(o.metadata, Some(id(50)));
    let created = o
        .info_entries
        .iter()
        .find(|e| e.key == "CreationDate")
        .unwrap();
    assert_eq!(created.date.unwrap().iso(), "2024-01-31T12:34:56+09:00");
    let mismatched: Vec<_> = o
        .issues
        .iter()
        .filter(|i| i.code == "InfoXmp")
        .map(|i| i.message.as_str())
        .collect();
    // Title and CreationDate agree; Producer and ModDate do not.
    assert_eq!(mismatched.len(), 2, "{mismatched:?}");
    assert!(mismatched.iter().any(|m| m.contains("/Producer")));
    assert!(mismatched.iter().any(|m| m.contains("/ModDate")));
    let keys: Vec<_> = o.features.iter().map(|f| f.key.as_str()).collect();
    assert!(
        keys.contains(&"Names/Dests") && keys.contains(&"PageLabels"),
        "{keys:?}"
    );
    let open_action = o.features.iter().find(|f| f.key == "OpenAction").unwrap();
    assert_eq!(open_action.text, "page 1");
    let x = m.metadata().unwrap();
    assert!(x.xmp.nodes.iter().any(|n| n.name == "dc:title"));
}

#[test]
fn inventory() {
    let f = open("model.pdf");
    let dm = DocModel::new();
    let m = dm.on(&f);
    let mut b = Builder::new(&m);
    let total = b.total();
    let mut steps = 0;
    while b.step(&m, 7, 2) {
        steps += 1;
        assert!(b.done() <= total);
    }
    assert!(steps > 3);
    assert_eq!(b.done(), total);
    let inv = b.finish();

    let font = |n| inv.fonts.iter().find(|f| f.id == id(n)).unwrap();
    assert_eq!(font(20).pages, [0, 1, 2, 3, 4]);
    assert!(font(20).standard && font(20).embedded.is_none());
    let tt = font(21);
    assert!(tt.subset && tt.to_unicode);
    assert_eq!(tt.embedded.as_deref(), Some("FontFile2"));
    assert_eq!(
        tt.encoding.as_deref(),
        Some("/WinAnsiEncoding + Differences")
    );
    assert_eq!(tt.pages, [0, 1, 2, 3]);
    assert_eq!(font(25).descendant, Some(id(26)));
    assert_eq!(font(25).embedded.as_deref(), Some("FontFile2"));
    // No /Type, found through the form XObject's resources.
    assert_eq!(font(28).pages, [4]);

    let img = |n| inv.images.iter().find(|i| i.id == id(n)).unwrap();
    assert_eq!(
        (img(60).width, img(60).color_space.as_deref()),
        (Some(2), Some("DeviceRGB"))
    );
    assert_eq!(img(61).color_space.as_deref(), Some("ICCBased (3)"));
    assert_eq!(
        (img(61).smask, img(61).pages.as_slice()),
        (Some(id(63)), &[4u32][..])
    );
    assert!(img(63).pages.is_empty());
    assert_eq!(inv.forms, 1);

    assert_eq!(inv.refs.referrers(1), [TRAILER]);
    assert_eq!(inv.refs.referrers(20), [2, 14]);
    assert_eq!(inv.refs.referrers(12), [2, 10, 41]);
    assert_eq!(inv.unreferenced, [id(90)]);
    let page10 = f.get(id(10)).unwrap();
    assert_eq!(
        ref_paths(&page10.node.obj, id(12), 8),
        ["/Annots[0]/Dest[0]"]
    );
}

#[test]
fn simple_file() {
    let f = open("base.pdf");
    let dm = DocModel::new();
    let m = dm.on(&f);
    assert_eq!(m.page_count(), 3);
    let p = m.page_info(2).unwrap();
    assert_eq!(p.page.id, id(5));
    assert!(p.issues.is_empty(), "{:?}", p.issues);
    assert_eq!(p.label, None);
    assert!(m.issues().is_empty());
    assert_eq!(m.outline_children(None).len(), 0);
}
