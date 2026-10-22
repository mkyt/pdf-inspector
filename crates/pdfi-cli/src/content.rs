//! `pdfi ops`: the operations of a content stream.

use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use pdfi_content::interp::{Color, State};
use pdfi_content::op::Op;
use pdfi_content::{Analysis, DocContent, Issue, Target};
use pdfi_model::{DocModel, Rect};
use pdfi_syntax::{Object, PdfFile, write};

fn rect_text(r: &Rect) -> String {
    format!("[{:.2} {:.2} {:.2} {:.2}]", r.x0, r.y0, r.x1, r.y1)
}

fn operand_text(o: &Object) -> String {
    let mut out = Vec::new();
    write::write_object(o, &mut out);
    let s = String::from_utf8_lossy(&out).into_owned();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

fn print_issue(i: &Issue) {
    let at = i.op.map(|o| format!("#{o} ")).unwrap_or_default();
    println!(
        "  {:<7} {:<18} {at}{}",
        format!("{:?}", i.severity).to_lowercase(),
        i.code,
        i.message
    );
}

fn color_text(c: &Color) -> String {
    let comps: Vec<String> = c.components.iter().map(|v| format!("{v}")).collect();
    let pat = c
        .pattern
        .as_ref()
        .map(|p| format!(" /{p}"))
        .unwrap_or_default();
    format!("{} [{}]{pat}", c.space, comps.join(" "))
}

fn print_state(label: &str, s: &State) {
    let g = &s.g;
    let t = &g.text;
    println!("{label}:");
    println!("  CTM        {:?}", g.ctm);
    println!(
        "  clip       {}",
        g.clip.as_ref().map_or("none".into(), rect_text)
    );
    println!(
        "  saved      {}  text object: {}  marked: {:?}",
        s.stack.len(),
        s.in_text,
        s.marked
    );
    println!(
        "  stroke     {}  alpha {}",
        color_text(&g.stroke),
        g.stroke_alpha
    );
    println!(
        "  fill       {}  alpha {}",
        color_text(&g.fill),
        g.fill_alpha
    );
    println!(
        "  line       width {} cap {} join {} miter {} dash {:?} {}",
        g.line_width, g.line_cap, g.line_join, g.miter_limit, g.dash, g.dash_phase
    );
    if !g.ext_gstates.is_empty() {
        println!(
            "  gs         {:?}  blend {}  soft mask {:?}",
            g.ext_gstates, g.blend, g.soft_mask
        );
    }
    match &t.font {
        Some(f) => println!(
            "  font       /{} {} ({})",
            f.name,
            t.size,
            f.info.base_font.as_deref().unwrap_or("?")
        ),
        None => println!("  font       none"),
    }
    println!(
        "  text       Tc {} Tw {} Tz {} TL {} Tr {} Ts {}",
        t.char_spacing, t.word_spacing, t.scale, t.leading, t.render, t.rise
    );
    if s.in_text {
        println!("  Tm         {:?}", s.tm);
    }
}

fn print_op(a: &Analysis, i: usize, op: &Op) {
    let idx = i as u32;
    let mut line = "  ".repeat(a.depth[i] as usize);
    for o in &op.operands {
        line.push_str(&operand_text(&o.obj));
        line.push(' ');
    }
    if let Some(img) = &op.inline_image {
        line.push_str(&format!(
            "{} ID <{} bytes> EI ",
            operand_text(&Object::Dict(img.dict.clone())),
            img.data.len()
        ));
    }
    line.push_str(&op.operator.name());
    let mut extra = Vec::new();
    if let Some(t) = a.text_of(idx) {
        extra.push(format!("“{t}”"));
    }
    if let Some(e) = a.element_of(idx) {
        extra.push(format!(
            "{} {}{}",
            e.kind.name(),
            rect_text(&e.bbox),
            if e.hidden { " hidden" } else { "" }
        ));
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("    % {}", extra.join("  "))
    };
    println!("{i:>7}  {line}{extra}");
    for x in a.issues_of(idx) {
        println!("         ! {:?} {}: {}", x.severity, x.code, x.message);
    }
}

pub fn ops(
    file: &PdfFile,
    page: Option<u64>,
    obj: Option<u32>,
    from: usize,
    count: Option<usize>,
    state: Option<usize>,
    issues_only: bool,
) -> Result<()> {
    let dm = DocModel::new();
    let dc = DocContent::new();
    let c = dc.on(dm.on(file));
    let target = match (page, obj) {
        (Some(p), None) if p > 0 => Target::Page(p - 1),
        (None, Some(n)) => Target::Stream(file.get_num(n)?.id),
        _ => bail!("give a page number (1-based) or --obj N"),
    };
    let t0 = Instant::now();
    let a = c.analyze(target).map_err(|e| anyhow!(e))?;
    let n = a.ops;
    for (k, s) in a.sources.iter().enumerate() {
        let id = s.id.map(|i| format!("{i} obj")).unwrap_or("-".into());
        let err = s
            .error
            .as_ref()
            .map(|e| format!("  error: {e}"))
            .unwrap_or_default();
        println!("source [{k}] {id}  {} bytes{err}", s.len);
    }
    println!(
        "{n} operations, {} painting, {} issues  ({:.1} ms){}",
        a.elements.len(),
        a.issues.len(),
        t0.elapsed().as_secs_f64() * 1000.0,
        if a.approximate {
            "  (some text boxes use guessed widths)"
        } else {
            ""
        }
    );
    if let Some(b) = &a.bounds {
        println!("bounds {}", rect_text(b));
    }
    if let Some(i) = state {
        let (before, after) = c
            .state_at(&a, i)
            .ok_or_else(|| anyhow!("no operation {i}; the content has {n}"))?;
        print_op(&a, i, &a.read_ops(i, 1)[0]);
        print_state("before", &before);
        print_state("after", &after);
        return Ok(());
    }
    if issues_only {
        a.issues.iter().for_each(print_issue);
        return Ok(());
    }
    for x in a.issues.iter().filter(|x| x.op.is_none()) {
        print_issue(x);
    }
    let end = count.map_or(n, |c| from.saturating_add(c).min(n));
    let from = from.min(n);
    for (k, op) in a.read_ops(from, end - from).iter().enumerate() {
        print_op(&a, from + k, op);
    }
    Ok(())
}
