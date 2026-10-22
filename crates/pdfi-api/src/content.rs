//! Requests about content streams (pdfi-content): the instruction list,
//! its problems, what each operation paints and the state around it.

use std::sync::Arc;

use pdfi_content::interp::{self, Element, State};
use pdfi_content::op::{Op, Operator};
use pdfi_content::{Analysis, Target};
use pdfi_syntax::{self as syn, DictExt, Object};

use crate::dto::*;
use crate::model::rect;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, page};

/// Characters of one operand shown in an [`OpView`].
const MAX_OPERAND: usize = 200;
/// Operands shown per operation.
const MAX_OPERANDS: usize = 32;

fn target(c: ContentRef) -> Target {
    match c {
        ContentRef::Page { index } => Target::Page(index.into()),
        ContentRef::Stream { id } => Target::Stream(id.into()),
    }
}

fn analysis(d: &Doc, c: ContentRef) -> Result<Arc<Analysis>, ApiError> {
    d.content
        .on(d.model.on(&d.file))
        .analyze(target(c))
        .map_err(|e| ApiError::new(ErrorCode::NotFound, e))
}

fn severity(s: syn::Severity) -> Severity {
    match s {
        syn::Severity::Info => Severity::Info,
        syn::Severity::Warning => Severity::Warning,
        syn::Severity::Error => Severity::Error,
    }
}

fn issue(i: &pdfi_content::Issue) -> ContentIssue {
    ContentIssue {
        op: i.op,
        severity: severity(i.severity),
        code: i.code.into(),
        message: i.message.clone(),
        source: i.at.map(|a| a.source),
        span: i.at.map(|a| a.span.into()),
    }
}

fn element(e: &Element) -> ElementView {
    ElementView {
        op: e.op,
        kind: e.kind.name().into(),
        bbox: rect(e.bbox),
        hidden: e.hidden,
    }
}

pub fn summary(d: &Doc, c: ContentRef) -> Result<ContentSummary, ApiError> {
    let a = analysis(d, c)?;
    let count = |s| a.issues.iter().filter(|i| i.severity == s).count() as u64;
    Ok(ContentSummary {
        content: c,
        kind: match a.kind {
            interp::Kind::Page => ContentKind::Page,
            interp::Kind::Form => ContentKind::Form,
            interp::Kind::Pattern => ContentKind::Pattern,
            interp::Kind::Other => ContentKind::Other,
        },
        id: a.id.into(),
        sources: a
            .sources
            .iter()
            .map(|s| ContentSourceInfo {
                id: s.id.map(Into::into),
                len: s.len,
                error: s.error.clone(),
            })
            .collect(),
        ops: a.ops as u64,
        truncated: a.truncated,
        elements: a.elements.len() as u64,
        issues: a.issues.len() as u64,
        errors: count(syn::Severity::Error),
        warnings: count(syn::Severity::Warning),
        issues_dropped: a.issues_dropped,
        bounds: a.bounds.map(rect),
        resources: a.resources_id.map(Into::into),
        has_resources: a.resources.as_ref().and_then(Object::as_dict).is_some(),
        approximate: a.approximate,
        counts: a
            .counts
            .iter()
            .map(|(c, n)| CategoryCount {
                category: c.name().into(),
                count: *n,
            })
            .collect(),
    })
}

/// An operand in PDF syntax, shortened to about `max` characters.
fn operand_text(o: &Object, max: usize, out: &mut String) {
    if out.len() > max {
        return;
    }
    match o {
        Object::String(s) => {
            let printable = s
                .bytes
                .iter()
                .all(|&b| (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\t');
            if printable && s.kind == syn::StrKind::Literal {
                out.push('(');
                for &b in &s.bytes {
                    match b {
                        b'(' | b')' | b'\\' => {
                            out.push('\\');
                            out.push(b as char);
                        }
                        b'\n' => out.push_str("\\n"),
                        b'\t' => out.push_str("\\t"),
                        _ => out.push(b as char),
                    }
                    if out.len() > max {
                        break;
                    }
                }
                out.push(')');
            } else {
                out.push('<');
                for b in &s.bytes {
                    out.push_str(&format!("{b:02x}"));
                    if out.len() > max {
                        break;
                    }
                }
                out.push('>');
            }
        }
        Object::Array(a) => {
            out.push('[');
            for (i, n) in a.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                if out.len() > max {
                    out.push('…');
                    break;
                }
                operand_text(&n.obj, max, out);
            }
            out.push(']');
        }
        Object::Dict(d) => {
            out.push_str("<<");
            for (k, v) in d {
                if out.len() > max {
                    out.push_str(" …");
                    break;
                }
                out.push(' ');
                let mut b = Vec::new();
                syn::write::write_name(k, &mut b);
                out.push_str(&String::from_utf8_lossy(&b));
                out.push(' ');
                operand_text(&v.obj, max, out);
            }
            out.push_str(" >>");
        }
        other => {
            let mut b = Vec::new();
            syn::write::write_object(other, &mut b);
            out.push_str(&String::from_utf8_lossy(&b));
        }
    }
}

fn shorten(mut s: String) -> String {
    if s.len() > MAX_OPERAND + 8 {
        let mut cut = MAX_OPERAND;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// The indirect object a resource-naming operation refers to.
fn resource_of(file: &syn::PdfFile, a: &Analysis, op: &Op) -> Option<ObjRef> {
    let category = match op.operator {
        Operator::Font => "Font",
        Operator::XObject => "XObject",
        Operator::ExtGState => "ExtGState",
        Operator::Shade => "Shading",
        Operator::StrokeSpace | Operator::FillSpace => "ColorSpace",
        Operator::StrokeColorN | Operator::FillColorN => "Pattern",
        Operator::MarkPointProps | Operator::BeginMarkedProps => "Properties",
        _ => return None,
    };
    let name = match op.operator {
        // A property list given inline names nothing.
        Operator::MarkPointProps | Operator::BeginMarkedProps => {
            op.operands.last()?.obj.as_name()?
        }
        _ => op.operands.iter().rev().find_map(|n| n.obj.as_name())?,
    };
    let res = a.resources.as_ref()?.as_dict()?;
    let (_, entries) = pdfi_model::resolve(file, res.get_key(category)?);
    entries
        .as_dict()?
        .get(name.as_bytes())?
        .obj
        .as_ref_id()
        .map(Into::into)
}

fn op_view(file: &syn::PdfFile, a: &Analysis, i: usize, op: &Op) -> OpView {
    let idx = i as u32;
    let mut operands: Vec<String> = op
        .operands
        .iter()
        .take(MAX_OPERANDS)
        .map(|n| {
            let mut s = String::new();
            operand_text(&n.obj, MAX_OPERAND, &mut s);
            shorten(s)
        })
        .collect();
    if op.operands.len() > MAX_OPERANDS {
        operands.push(format!("… {} more", op.operands.len() - MAX_OPERANDS));
    }
    if let Some(img) = &op.inline_image {
        let mut s = String::new();
        operand_text(&Object::Dict(img.dict.clone()), MAX_OPERAND, &mut s);
        operands.push(shorten(s));
        operands.push(format!("ID … {} bytes … EI", img.data.len()));
    }
    OpView {
        index: idx,
        operator: op.operator.name().into_owned(),
        category: op.operator.category().name().into(),
        operands,
        depth: a.depth.get(i).copied().unwrap_or(0).into(),
        source: op.origin.map(|o| o.source),
        span: op.origin.map(|o| o.span.into()),
        text: a.text_of(idx).map(str::to_owned),
        resource: resource_of(file, a, op),
        element: a.element_of(idx).map(element),
        issues: a.issues_of(idx).iter().map(issue).collect(),
    }
}

pub fn ops(d: &Doc, c: ContentRef, range: Range) -> Result<Page<OpView>, ApiError> {
    let a = analysis(d, c)?;
    Ok(page(a.ops as u64, range, |o, n| {
        a.read_ops(o, n)
            .iter()
            .enumerate()
            .map(|(k, op)| op_view(&d.file, &a, o + k, op))
            .collect()
    }))
}

pub fn issues(d: &Doc, c: ContentRef, range: Range) -> Result<Page<ContentIssue>, ApiError> {
    let a = analysis(d, c)?;
    Ok(page(a.issues.len() as u64, range, |o, n| {
        a.issues[o..o + n].iter().map(issue).collect()
    }))
}

pub fn elements(d: &Doc, c: ContentRef, range: Range) -> Result<Page<ElementView>, ApiError> {
    let a = analysis(d, c)?;
    Ok(page(a.elements.len() as u64, range, |o, n| {
        a.elements[o..o + n].iter().map(element).collect()
    }))
}

fn color(c: &interp::Color) -> ColorView {
    ColorView {
        space: c.space.clone(),
        family: c.family.clone(),
        components: c.components.clone(),
        pattern: c.pattern.clone(),
    }
}

fn graphics_state(s: &State) -> GraphicsState {
    let g = &s.g;
    let t = &g.text;
    GraphicsState {
        ctm: g.ctm.to_vec(),
        clip: g.clip.map(rect),
        line_width: g.line_width,
        line_cap: g.line_cap,
        line_join: g.line_join,
        miter_limit: g.miter_limit,
        dash: g.dash.clone(),
        dash_phase: g.dash_phase,
        intent: g.intent.clone(),
        flatness: g.flatness,
        stroke: color(&g.stroke),
        fill: color(&g.fill),
        stroke_alpha: g.stroke_alpha,
        fill_alpha: g.fill_alpha,
        blend: g.blend.clone(),
        soft_mask: g.soft_mask.clone(),
        ext_gstates: g.ext_gstates.clone(),
        font: t.font.as_ref().map(|f| FontView {
            name: f.name.clone(),
            id: f.id.map(Into::into),
            base_font: f.info.base_font.clone(),
            subtype: f.info.subtype.clone(),
            approximate: f.info.approximate,
        }),
        font_size: t.size,
        char_spacing: t.char_spacing,
        word_spacing: t.word_spacing,
        horizontal_scale: t.scale,
        leading: t.leading,
        render_mode: t.render,
        rise: t.rise,
        in_text: s.in_text,
        text_matrix: s.in_text.then(|| s.tm.to_vec()),
        line_matrix: s.in_text.then(|| s.tlm.to_vec()),
        saved: (s.stack.len() + s.stack_overflow as usize) as u32,
        marked: s.marked.clone(),
        path: s.path.as_ref().and_then(|p| p.bbox).map(rect),
        current_point: s
            .path
            .as_ref()
            .and_then(|p| p.current)
            .map(|(x, y)| vec![x, y]),
    }
}

pub fn op_state(d: &Doc, c: ContentRef, index: u32) -> Result<OpState, ApiError> {
    let a = analysis(d, c)?;
    let content = d.content.on(d.model.on(&d.file));
    let (before, after) = content.state_at(&a, index as usize).ok_or_else(|| {
        ApiError::new(
            ErrorCode::NotFound,
            format!("no operation {index} (the content has {})", a.ops),
        )
    })?;
    let op = a
        .read_ops(index as usize, 1)
        .pop()
        .ok_or_else(|| ApiError::new(ErrorCode::Internal, "operation not found again"))?;
    Ok(OpState {
        index,
        operator: op.operator.name().into_owned(),
        description: op.operator.description().into(),
        before: graphics_state(&before),
        after: graphics_state(&after),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(o: Object) -> String {
        let mut s = String::new();
        operand_text(&o, MAX_OPERAND, &mut s);
        s
    }

    #[test]
    fn operands_in_pdf_syntax() {
        let lit = |b: &[u8]| Object::String(syn::PdfString::literal(b.to_vec()));
        assert_eq!(text(lit(b"a(b)")), "(a\\(b\\))");
        assert_eq!(text(lit(b"\x00\x41")), "<0041>");
        let arr = Object::Array(vec![
            lit(b"A").into(),
            Object::Integer(-20).into(),
            Object::Real(0.5).into(),
        ]);
        assert_eq!(text(arr), "[(A) -20 0.5]");
        assert_eq!(shorten("x".repeat(500)).chars().count(), MAX_OPERAND + 1);
    }
}
