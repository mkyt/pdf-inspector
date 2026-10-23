//! Annotations of a page: what they say, where they are (`/Rect`,
//! `/QuadPoints`, ink and vertex paths) and how their appearance stream is
//! put on the page (ISO 32000-2 12.5.5).

use pdfi_syntax::{DictExt, ObjId, PdfFile};

use crate::Model;
use crate::dest::Link;
use crate::text::pdf_date;
use crate::util::{Rect, get, int, name, name_str, numbers, resolve, text};

/// `/F` bits.
pub const INVISIBLE: i64 = 1;
pub const HIDDEN: i64 = 1 << 1;
pub const PRINT: i64 = 1 << 2;
pub const NO_ZOOM: i64 = 1 << 3;
pub const NO_ROTATE: i64 = 1 << 4;
pub const NO_VIEW: i64 = 1 << 5;

/// Names of the `/F` bits, lowest first.
pub const FLAG_NAMES: [&str; 10] = [
    "Invisible",
    "Hidden",
    "Print",
    "NoZoom",
    "NoRotate",
    "NoView",
    "ReadOnly",
    "Locked",
    "ToggleNoView",
    "LockedContents",
];

/// A 2D affine matrix `[a b c d e f]` (points map as row vectors).
pub type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n`.
fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

/// The bounding box of `r` transformed by `m`.
fn bounds(r: &Rect, m: &Matrix) -> Rect {
    let pts = [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)]
        .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    let (xs, ys) = (pts.map(|p| p.0), pts.map(|p| p.1));
    Rect {
        x0: xs.iter().copied().fold(f64::INFINITY, f64::min),
        y0: ys.iter().copied().fold(f64::INFINITY, f64::min),
        x1: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        y1: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

/// One appearance stream of an annotation's `/AP`, and how it is put on
/// the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    /// The `/AP` entry: `N` (normal), `R` (rollover) or `D` (down).
    pub kind: String,
    /// The form XObject (`None` when it is a direct object, which cannot be).
    pub id: Option<ObjId>,
    /// The state, when the entry is a dictionary of states (the one `/AS`
    /// chooses, for the normal appearance).
    pub state: Option<String>,
    /// All states of the entry, when it is a dictionary.
    pub states: Vec<String>,
    /// The form's `/BBox`.
    pub bbox: Option<Rect>,
    /// The form's `/Matrix`.
    pub matrix: Matrix,
    /// Form space to the page's default space: `/Matrix` then the matrix
    /// that fits the transformed `/BBox` to `/Rect` (ISO 32000-2 12.5.5).
    pub to_page: Option<Matrix>,
    /// Why it cannot be painted.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annot {
    /// Position in the page's `/Annots`.
    pub index: usize,
    pub id: Option<ObjId>,
    pub subtype: Option<String>,
    pub rect: Option<Rect>,
    pub contents: Option<String>,
    /// `/NM`.
    pub name: Option<String>,
    /// `/F` (Invisible, Hidden, Print, ...).
    pub flags: i64,
    /// `/T`: who wrote it (markup annotations).
    pub author: Option<String>,
    /// `/Subj`.
    pub subject: Option<String>,
    /// `/M` (ISO 8601 when it is a PDF date, else as written).
    pub modified: Option<String>,
    /// `/C`: 0 (transparent), 1 (gray), 3 (RGB) or 4 (CMYK) components.
    pub color: Option<Vec<f64>>,
    /// `/IC`: the interior color.
    pub interior: Option<Vec<f64>>,
    /// `/CA`.
    pub opacity: Option<f64>,
    /// Border width: `/BS /W`, or the third number of `/Border`.
    pub border: Option<f64>,
    /// `/QuadPoints`, 8 numbers each (x1 y1 … x4 y4).
    pub quads: Vec<[f64; 8]>,
    /// `/InkList` paths, `/Vertices` (Polygon, PolyLine) or `/L` (Line).
    pub paths: Vec<Vec<(f64, f64)>>,
    /// The `/AP` entries present (`N`, `R`, `D`).
    pub appearances: Vec<String>,
    /// The normal appearance (by `/AS` when `/N` has states), when there
    /// is one.
    pub appearance: Option<Appearance>,
    /// Every appearance stream of `/AP` (`N`, `R`, `D`, each state), for
    /// looking at the ones a viewer shows only on interaction.
    pub variants: Vec<Appearance>,
    /// `/AS`.
    pub r#as: Option<String>,
    /// Where a link (or a widget's action) goes.
    pub link: Option<Link>,
    /// `/Parent` of a pop-up, `/IRT` of a reply.
    pub parent: Option<ObjId>,
    /// `/Popup`.
    pub popup: Option<ObjId>,
    pub error: Option<String>,
}

impl Annot {
    fn new(index: usize, id: Option<ObjId>) -> Self {
        Annot {
            index,
            id,
            subtype: None,
            rect: None,
            contents: None,
            name: None,
            flags: 0,
            author: None,
            subject: None,
            modified: None,
            color: None,
            interior: None,
            opacity: None,
            border: None,
            quads: Vec::new(),
            paths: Vec::new(),
            appearances: Vec::new(),
            appearance: None,
            variants: Vec::new(),
            r#as: None,
            link: None,
            parent: None,
            popup: None,
            error: None,
        }
    }

    /// Whether a viewer shows it on screen: not Hidden or NoView, and
    /// (when it has no appearance) not Invisible.
    pub fn shown(&self) -> bool {
        self.flags & (HIDDEN | NO_VIEW) == 0
            && (self.appearance.is_some() || self.flags & INVISIBLE == 0)
    }

    /// The appearance stream to paint and its matrix to the page, when it
    /// is shown and can be painted.
    pub fn paint(&self) -> Option<(ObjId, Matrix)> {
        let a = self.appearance.as_ref()?;
        if !self.shown() || a.error.is_some() {
            return None;
        }
        Some((a.id?, a.to_page?))
    }

    /// Another appearance stream to paint instead (`/AP` entry `kind`,
    /// state `state`), when it is shown and that one can be painted.
    pub fn paint_variant(&self, kind: &str, state: Option<&str>) -> Option<(ObjId, Matrix)> {
        let a = self
            .variants
            .iter()
            .find(|v| v.kind == kind && v.state.as_deref() == state)?;
        if !self.shown() || a.error.is_some() {
            return None;
        }
        Some((a.id?, a.to_page?))
    }
}

/// Points `(x, y)` from a flat list of numbers.
fn points(v: &[f64]) -> Vec<(f64, f64)> {
    v.chunks_exact(2).map(|p| (p[0], p[1])).collect()
}

/// An appearance stream (`r`, object `id`) put on `rect`.
fn form(
    f: &PdfFile,
    id: Option<ObjId>,
    r: &pdfi_syntax::Object,
    kind: &str,
    state: Option<String>,
    states: Vec<String>,
    rect: Option<Rect>,
) -> Appearance {
    let mut a = Appearance {
        kind: kind.into(),
        id,
        state,
        states,
        bbox: None,
        matrix: IDENTITY,
        to_page: None,
        error: None,
    };
    let Some(s) = r.as_stream() else {
        a.error = Some(format!(
            "the appearance is not a stream ({})",
            r.type_name()
        ));
        return a;
    };
    a.bbox = s.dict.get_key("BBox").and_then(|b| Rect::read(f, b));
    a.matrix = s
        .dict
        .get_key("Matrix")
        .and_then(|m| numbers(f, m))
        .and_then(|m| <[f64; 6]>::try_from(m).ok())
        .unwrap_or(IDENTITY);
    if let (Some(b), Some(rect)) = (a.bbox, rect) {
        let t = bounds(&b, &a.matrix);
        let sx = if t.width() > 0.0 {
            rect.width() / t.width()
        } else {
            1.0
        };
        let sy = if t.height() > 0.0 {
            rect.height() / t.height()
        } else {
            1.0
        };
        let fit = [sx, 0.0, 0.0, sy, rect.x0 - t.x0 * sx, rect.y0 - t.y0 * sy];
        a.to_page = Some(mul(&a.matrix, &fit));
    }
    if a.bbox.is_none() {
        a.error = Some("the appearance stream has no valid /BBox".into());
    } else if rect.is_none() {
        a.error = Some("the annotation has no valid /Rect".into());
    } else if id.is_none() {
        a.error = Some("the appearance stream is a direct object".into());
    }
    a
}

/// Every appearance stream of `/AP`, and the normal one chosen by `/AS`.
fn appearances(
    f: &PdfFile,
    d: &pdfi_syntax::Dict,
    rect: Option<Rect>,
    r#as: Option<&str>,
) -> (Option<Appearance>, Vec<Appearance>) {
    let Some(ap) = get(f, d, "AP") else {
        return (None, Vec::new());
    };
    let Some(ap) = ap.as_dict() else {
        return (None, Vec::new());
    };
    let mut normal = None;
    let mut all = Vec::new();
    for kind in ["N", "R", "D"] {
        let Some(v) = ap.get_key(kind) else { continue };
        let (id, r) = resolve(f, v);
        match r.as_dict().filter(|_| r.as_stream().is_none()) {
            None => {
                let a = form(f, id, &r, kind, None, Vec::new(), rect);
                if kind == "N" {
                    normal = Some(a.clone());
                }
                all.push(a);
            }
            Some(sd) => {
                let states: Vec<String> = sd.iter().map(|(k, _)| name_str(k)).collect();
                for (k, v) in sd {
                    let (id, r) = resolve(f, &v.obj);
                    let a = form(f, id, &r, kind, Some(name_str(k)), states.clone(), rect);
                    if kind == "N" && r#as == Some(name_str(k).as_str()) {
                        normal = Some(a.clone());
                    }
                    all.push(a);
                }
                if kind == "N" && normal.is_none() {
                    normal = Some(Appearance {
                        kind: kind.into(),
                        id: None,
                        state: r#as.map(Into::into),
                        states,
                        bbox: None,
                        matrix: IDENTITY,
                        to_page: None,
                        error: Some(match r#as {
                            None => "/AP /N has states but there is no /AS to choose one".into(),
                            Some(s) => format!("/AS /{s} is not a state of /AP /N"),
                        }),
                    });
                }
            }
        }
    }
    (normal, all)
}

impl Model<'_> {
    /// Annotations `offset..offset + limit` of page `index`, and how many the
    /// page has.
    pub fn annots(
        &self,
        index: u64,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<Annot>), String> {
        let f = self.file;
        let p = self.page(index)?;
        let page = f.get(p.id).map_err(|e| e.to_string())?;
        let Some(arr) = page.node.obj.as_dict().and_then(|d| get(f, d, "Annots")) else {
            return Ok((0, Vec::new()));
        };
        let items = arr.as_array().unwrap_or_default();
        let out = items
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(i, n)| {
                let (id, r) = resolve(f, &n.obj);
                let mut a = Annot::new(i, id);
                let Some(d) = r.as_dict() else {
                    a.error = Some(format!("not a dictionary ({})", r.type_name()));
                    return a;
                };
                a.subtype = name(f, d, "Subtype").map(|n| name_str(&n));
                a.rect = d.get_key("Rect").and_then(|r| Rect::read(f, r));
                a.contents = text(f, d, "Contents");
                a.name = text(f, d, "NM");
                a.flags = int(f, d, "F").unwrap_or(0);
                a.author = text(f, d, "T");
                a.subject = text(f, d, "Subj");
                a.modified = get(f, d, "M").and_then(|m| {
                    let s = m.as_string()?;
                    Some(
                        pdf_date(&s.bytes)
                            .map(|d| d.iso())
                            .unwrap_or_else(|| crate::text::decode(&s.bytes)),
                    )
                });
                a.color = d.get_key("C").and_then(|c| numbers(f, c));
                a.interior = d.get_key("IC").and_then(|c| numbers(f, c));
                a.opacity = get(f, d, "CA").and_then(|v| v.as_f64());
                a.border = get(f, d, "BS")
                    .and_then(|bs| bs.as_dict().and_then(|bs| get(f, bs, "W")?.as_f64()))
                    .or_else(|| {
                        d.get_key("Border")
                            .and_then(|b| numbers(f, b))
                            .and_then(|b| b.get(2).copied())
                    });
                a.quads = d
                    .get_key("QuadPoints")
                    .and_then(|q| numbers(f, q))
                    .map(|q| {
                        q.chunks_exact(8)
                            .map(|c| <[f64; 8]>::try_from(c).expect("8 numbers"))
                            .collect()
                    })
                    .unwrap_or_default();
                a.paths = paths(f, d, a.subtype.as_deref());
                a.appearances = get(f, d, "AP")
                    .and_then(|ap| {
                        ap.as_dict()
                            .map(|ap| ap.iter().map(|(k, _)| name_str(k)).collect())
                    })
                    .unwrap_or_default();
                a.r#as = name(f, d, "AS").map(|n| name_str(&n));
                (a.appearance, a.variants) = appearances(f, d, a.rect, a.r#as.as_deref());
                a.link = self.link(d);
                a.parent = d
                    .get_key("Parent")
                    .or_else(|| d.get_key("IRT"))
                    .and_then(|p| p.as_ref_id());
                a.popup = d.get_key("Popup").and_then(|p| p.as_ref_id());
                a
            })
            .collect();
        Ok((items.len(), out))
    }
}

/// The geometry an annotation draws along, by its subtype.
fn paths(f: &PdfFile, d: &pdfi_syntax::Dict, subtype: Option<&str>) -> Vec<Vec<(f64, f64)>> {
    let flat = |k: &str| d.get_key(k).and_then(|v| numbers(f, v));
    match subtype {
        Some("Ink") => get(f, d, "InkList")
            .and_then(|l| {
                l.as_array().map(|l| {
                    l.iter()
                        .filter_map(|p| numbers(f, &p.obj))
                        .map(|p| points(&p))
                        .collect()
                })
            })
            .unwrap_or_default(),
        Some("Polygon") | Some("PolyLine") => flat("Vertices")
            .map(|v| vec![points(&v)])
            .unwrap_or_default(),
        Some("Line") => flat("L")
            .filter(|l| l.len() == 4)
            .map(|l| vec![points(&l)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}
