//! Commands over the semantic layer: pages, outline, metadata, inventory.

use std::time::Instant;

use anyhow::{Result, bail};
use pdfi_model::dest::Link;
use pdfi_model::inventory::{Builder, TRAILER, ref_paths};
use pdfi_model::pages::Origin;
use pdfi_model::{DocModel, Issue, Model, Rect};
use pdfi_syntax::{ObjId, Object, PdfFile};

fn rect_text(r: Option<Rect>) -> String {
    match r {
        Some(r) => format!("[{} {} {} {}]", r.x0, r.y0, r.x1, r.y1),
        None => "-".into(),
    }
}

fn origin_text(o: Origin) -> String {
    match o {
        Origin::Page => String::new(),
        Origin::Inherited(id) => format!("  (from {id})"),
        Origin::Default(of) => format!("  (default: {of})"),
        Origin::Missing => "  (missing)".into(),
    }
}

fn link_text(l: &Option<Link>) -> String {
    l.as_ref()
        .map(|l| format!(" → {}", l.describe()))
        .unwrap_or_default()
}

fn print_issues(issues: &[Issue]) {
    for i in issues {
        let at = i.obj.map(|id| format!(" [{id}]")).unwrap_or_default();
        println!(
            "  {:<7} {:<9} {}{at}",
            format!("{:?}", i.severity).to_lowercase(),
            i.code,
            i.message
        );
    }
}

pub fn pages(file: &PdfFile, from: u64, count: Option<u64>) -> Result<()> {
    let dm = DocModel::new();
    let m = dm.on(file);
    let n = m.page_count();
    let end = count.map_or(n, |c| (from + c).min(n));
    println!("{n} pages");
    for i in from..end {
        match m.page_summary(i) {
            Ok((p, size, rotate, annots)) => {
                let label = m.label(i).map(|l| format!(" ({l})")).unwrap_or_default();
                let size = size.map_or("-".into(), |r| format!("{}×{}", r.width(), r.height()));
                println!(
                    "{:>6}{label}  {:<10} {size} pt{}{}",
                    i + 1,
                    format!("{} obj", p.id),
                    if rotate != 0 {
                        format!("  rotate {rotate}")
                    } else {
                        String::new()
                    },
                    if annots > 0 {
                        format!("  {annots} annots")
                    } else {
                        String::new()
                    },
                );
            }
            Err(e) => println!("{:>6}  error: {e}", i + 1),
        }
    }
    print_issues(&m.issues());
    Ok(())
}

pub fn page(file: &PdfFile, number: u64) -> Result<()> {
    let dm = DocModel::new();
    let m = dm.on(file);
    if number == 0 {
        bail!("pages are numbered from 1");
    }
    let t0 = Instant::now();
    let p = m.page_info(number - 1).map_err(anyhow::Error::msg)?;
    let found = t0.elapsed();
    println!(
        "page:      {number} of {} ({} obj)",
        m.page_count(),
        p.page.id
    );
    if let Some(l) = &p.label {
        println!("label:     {l}");
    }
    let path: Vec<String> = p.page.ancestors.iter().map(ToString::to_string).collect();
    println!("tree:      {} (found in {found:.2?})", path.join(" → "));
    for b in &p.boxes {
        println!(
            "{:<10} {}{}",
            format!("{}:", b.name),
            rect_text(b.rect),
            origin_text(b.origin)
        );
    }
    println!("rotate:    {}{}", p.rotate, origin_text(p.rotate_origin));
    if p.user_unit != 1.0 {
        println!("userunit:  {}", p.user_unit);
    }
    let res = p
        .resources_id
        .map(|id| format!("{id} obj"))
        .unwrap_or("direct".into());
    println!("resources: {res}{}", origin_text(p.resources_origin));
    if let Some(Object::Dict(d)) = &p.resources {
        for (cat, v) in d {
            let (_, r) = pdfi_model::resolve(file, &v.obj);
            for (name, v) in r.as_dict().into_iter().flatten() {
                println!(
                    "  /{}/{}  {}",
                    cat.as_str_lossy(),
                    name.as_str_lossy(),
                    pdfi_model::inventory::describe(file, &v.obj)
                );
            }
        }
    }
    for c in &p.contents {
        let id =
            c.id.map(|id| format!("{id} obj"))
                .unwrap_or("direct".into());
        match &c.error {
            Some(e) => println!("contents:  {id}: {e}"),
            None => println!(
                "contents:  {id}  {} bytes {}",
                c.raw_len.unwrap_or(0),
                c.filters.join(" ")
            ),
        }
    }
    let (n, annots) = m.annots(number - 1, 0, 1000).map_err(anyhow::Error::msg)?;
    if n > 0 {
        println!("annots:    {n}");
        for a in annots {
            let id =
                a.id.map(|id| format!("{id} obj"))
                    .unwrap_or("direct".into());
            println!(
                "  [{}] {id} /{} {}{}{}",
                a.index,
                a.subtype.as_deref().unwrap_or("?"),
                rect_text(a.rect),
                a.contents.map(|c| format!(" ({c})")).unwrap_or_default(),
                link_text(&a.link),
            );
        }
    }
    print_issues(&p.issues);
    print_issues(&m.issues());
    Ok(())
}

pub fn outline(file: &PdfFile) -> Result<()> {
    let dm = DocModel::new();
    let m = dm.on(file);
    fn walk(m: &Model, parent: Option<ObjId>, depth: usize, shown: &mut usize) {
        for &id in m.outline_children(parent).iter() {
            if *shown >= 10_000 || depth > 32 {
                return;
            }
            *shown += 1;
            match m.outline_item(id) {
                Ok(i) => {
                    println!(
                        "{}{}{}  [{id}]",
                        "  ".repeat(depth),
                        i.title,
                        link_text(&i.link)
                    );
                    if i.has_children {
                        walk(m, Some(id), depth + 1, shown);
                    }
                }
                Err(e) => println!("{}error: {e}", "  ".repeat(depth)),
            }
        }
    }
    let mut shown = 0;
    walk(&m, None, 0, &mut shown);
    if shown == 0 {
        println!("(no outline)");
    }
    print_issues(&m.issues());
    Ok(())
}

pub fn meta(file: &PdfFile) -> Result<()> {
    let dm = DocModel::new();
    let m = dm.on(file);
    let o = m.overview();
    println!("pages:     {}", o.page_count);
    if let Some(id) = o.info {
        println!("info:      {id} obj");
    }
    for e in &o.info_entries {
        let date = e
            .date
            .map(|d| format!("  = {}", d.iso()))
            .unwrap_or_default();
        println!("  /{:<14} {}{date}", e.key, e.text);
    }
    for f in &o.features {
        println!("{:<10} {}", format!("{}:", f.key), f.text);
    }
    if let Some(x) = m.metadata() {
        println!("metadata:  {} obj, {} bytes", x.id, x.len);
        for p in &x.xmp.properties {
            println!("  {:<20} {}", p.name, p.value);
        }
    }
    print_issues(&o.issues);
    Ok(())
}

pub fn inventory(file: &PdfFile, refs: Option<u32>) -> Result<()> {
    let dm = DocModel::new();
    let m = dm.on(file);
    let t0 = Instant::now();
    let mut b = Builder::new(&m);
    while b.step(&m, 256, 32) {}
    let inv = b.finish();
    let pages = |p: &[u32]| match p {
        [] => "unused".to_string(),
        [one] => format!("page {}", one + 1),
        [first, ..] => format!("{} pages from {}", p.len(), first + 1),
    };
    if let Some(num) = refs {
        let target = ObjId::new(num, file.xref().get(num).map_or(0, |e| e.generation()));
        for &s in inv.refs.referrers(num) {
            if s == TRAILER {
                let t = Object::Dict(file.trailer().clone());
                println!("trailer {}", ref_paths(&t, target, 8).join(" "));
            } else if let Ok(o) = file.get_num(s) {
                println!(
                    "{} obj {}",
                    o.id,
                    ref_paths(&o.node.obj, target, 8).join(" ")
                );
            }
        }
        return Ok(());
    }
    println!("fonts: {}", inv.fonts.len());
    for f in &inv.fonts {
        println!(
            "  {:<10} {:<13} {:<30} {:<24} {}{}  {}",
            format!("{} obj", f.id),
            f.subtype.as_deref().unwrap_or("?"),
            f.base_font.as_deref().unwrap_or("?"),
            f.embedded.as_deref().unwrap_or(if f.standard {
                "standard 14"
            } else {
                "not embedded"
            }),
            f.encoding.as_deref().unwrap_or("-"),
            if f.to_unicode { " ToUnicode" } else { "" },
            pages(&f.pages),
        );
    }
    println!("images: {}", inv.images.len());
    for i in &inv.images {
        println!(
            "  {:<10} {}×{} {} bpc {:<20} {:<16} {} bytes  {}",
            format!("{} obj", i.id),
            i.width.unwrap_or(0),
            i.height.unwrap_or(0),
            i.bits.unwrap_or(0),
            if i.image_mask {
                "ImageMask".into()
            } else {
                i.color_space.clone().unwrap_or("-".into())
            },
            i.filters.join(" "),
            i.raw_len,
            pages(&i.pages),
        );
    }
    println!(
        "{} objects ({} failed), {} pages, {} forms, {} references, {} unreferenced ({:.2?})",
        inv.objects,
        inv.failed,
        inv.pages,
        inv.forms,
        inv.refs.edges(),
        inv.unreferenced.len(),
        t0.elapsed()
    );
    if !inv.unreferenced.is_empty() {
        let shown: Vec<String> = inv
            .unreferenced
            .iter()
            .take(20)
            .map(|id| id.num.to_string())
            .collect();
        println!(
            "unreferenced: {}{}",
            shown.join(" "),
            if inv.unreferenced.len() > 20 {
                " …"
            } else {
                ""
            }
        );
    }
    print_issues(&m.issues());
    Ok(())
}
