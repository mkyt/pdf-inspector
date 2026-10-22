//! The interpreter: runs operations, tracking the graphics and text state,
//! checks them (operands, nesting, placement, resources) and records what
//! they paint as [`Element`]s with bounding boxes in the content's space
//! (default user space for pages).
//!
//! Bounding boxes are conservative: curves count their control points,
//! strokes grow by half the line width, glyphs span the font's ascent and
//! descent over their advance. Form XObjects are followed (depth- and
//! budget-limited) to bound what they paint; patterns and glyph procedures
//! are not.
//!
//! With a [`Device`] in its [`Rec`], it also records the geometry and hands
//! paths, glyph runs and images to the device as it goes (rendering).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use pdfi_model::{Rect, resolve};
use pdfi_syntax::{Dict, DictExt, Name, Node, ObjId, Object, PdfFile, Severity, Stream};

use crate::device::{ClipPath, Device, Glyph, GlyphRun, ImageData, PathEl};
use crate::font::FontInfo;
use crate::geom::{
    IDENTITY, Matrix, add_point, apply, grow, intersect, mul, scale_of, transform, translate,
    union, unit_square,
};
use crate::op::{Arg, Category, Op, Operator};
use crate::parse::{OpReader, SourceData};
use crate::{DocContent, Issue, Issues};

/// Form XObjects followed inside each other.
const MAX_FORM_DEPTH: u32 = 12;
/// Saved graphics states kept (`q` beyond this is counted, not saved).
const MAX_STACK: usize = 4096;
/// Operations interpreted inside form XObjects for one content.
pub const FORM_BUDGET: u64 = 2_000_000;
/// TJ adjustments at least this large (thousandths of an em, leftwards
/// negative) read as a space in decoded text.
const SPACE_ADJUST: f64 = -180.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    /// As set: `DeviceRGB`, `Pattern`, or a `/ColorSpace` resource name.
    pub space: String,
    /// `DeviceRGB`, `ICCBased`, `Separation`, ...
    pub family: String,
    /// Number of components, when known.
    pub n: Option<usize>,
    pub components: Vec<f64>,
    /// Pattern name (`scn` with a pattern color space).
    pub pattern: Option<String>,
    /// The color space object of a `/ColorSpace` resource (`None` for the
    /// device spaces and `Pattern` without a base).
    pub def: Option<Arc<Object>>,
}

impl Color {
    fn device(family: &str, components: Vec<f64>) -> Self {
        Color {
            space: family.into(),
            family: family.into(),
            n: Some(components.len()),
            components,
            pattern: None,
            def: None,
        }
    }

    fn gray() -> Self {
        Self::device("DeviceGray", vec![0.0])
    }
}

#[derive(Clone, Debug)]
pub struct FontRef {
    /// Resource name, or `ExtGState` for a font set by `gs`.
    pub name: String,
    pub id: Option<ObjId>,
    pub info: Arc<FontInfo>,
}

#[derive(Clone, Debug)]
pub struct TextState {
    pub char_spacing: f64,
    pub word_spacing: f64,
    /// Horizontal scaling, percent.
    pub scale: f64,
    pub leading: f64,
    pub font: Option<FontRef>,
    pub size: f64,
    pub render: i64,
    pub rise: f64,
}

/// The graphics state (what `q` saves and `Q` restores).
#[derive(Clone, Debug)]
pub struct GState {
    pub ctm: Matrix,
    /// Bounding box of the clipping path; `None`: not clipped.
    pub clip: Option<Rect>,
    /// The clipping paths themselves (only while drawing).
    pub clip_path: Option<Arc<ClipPath>>,
    pub line_width: f64,
    pub line_cap: i64,
    pub line_join: i64,
    pub miter_limit: f64,
    pub dash: Vec<f64>,
    pub dash_phase: f64,
    pub intent: String,
    pub flatness: f64,
    pub stroke: Color,
    pub fill: Color,
    pub stroke_alpha: f64,
    pub fill_alpha: f64,
    pub blend: String,
    /// `None` or a description of the soft mask.
    pub soft_mask: Option<String>,
    /// `/ExtGState` resources applied, in order.
    pub ext_gstates: Vec<String>,
    pub text: TextState,
}

impl GState {
    pub fn new(ctm: Matrix, clip: Option<Rect>) -> Self {
        GState {
            ctm,
            clip,
            clip_path: None,
            line_width: 1.0,
            line_cap: 0,
            line_join: 0,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            intent: "RelativeColorimetric".into(),
            flatness: 1.0,
            stroke: Color::gray(),
            fill: Color::gray(),
            stroke_alpha: 1.0,
            fill_alpha: 1.0,
            blend: "Normal".into(),
            soft_mask: None,
            ext_gstates: Vec::new(),
            text: TextState {
                char_spacing: 0.0,
                word_spacing: 0.0,
                scale: 100.0,
                leading: 0.0,
                font: None,
                size: 0.0,
                render: 0,
                rise: 0.0,
            },
        }
    }
}

/// The path being built (between `m` / `re` and a painting operator).
#[derive(Clone, Debug, Default)]
pub struct PathState {
    /// In the content's space.
    pub bbox: Option<Rect>,
    /// In user space.
    pub current: Option<(f64, f64)>,
    pub start: Option<(f64, f64)>,
    pub segments: u32,
    /// The elements (only while drawing).
    pub els: Vec<PathEl>,
}

/// Everything the interpreter tracks.
#[derive(Clone, Debug)]
pub struct State {
    pub g: GState,
    pub stack: Vec<GState>,
    /// `q` beyond [`MAX_STACK`].
    pub stack_overflow: u32,
    pub tm: Matrix,
    pub tlm: Matrix,
    pub in_text: bool,
    pub path: Option<PathState>,
    /// `W` (false) or `W*` (true) waiting for the painting operator.
    pub pending_clip: Option<bool>,
    /// Open marked-content tags.
    pub marked: Vec<String>,
    /// Open `BX` sections.
    pub compat: u32,
}

impl State {
    pub fn new(g: GState) -> Self {
        State {
            g,
            stack: Vec::new(),
            stack_overflow: 0,
            tm: IDENTITY,
            tlm: IDENTITY,
            in_text: false,
            path: None,
            pending_clip: None,
            marked: Vec::new(),
            compat: 0,
        }
    }

    /// Nesting depth: saved states, text object, marked content.
    pub fn nesting(&self) -> usize {
        self.stack.len()
            + self.stack_overflow as usize
            + usize::from(self.in_text)
            + self.marked.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Fill,
    Stroke,
    FillStroke,
    /// `W n`: a clipping path that is not painted.
    Clip,
    Text,
    Image,
    InlineImage,
    Form,
    Shading,
}

impl ElementKind {
    pub fn name(self) -> &'static str {
        match self {
            ElementKind::Fill => "fill",
            ElementKind::Stroke => "stroke",
            ElementKind::FillStroke => "fillStroke",
            ElementKind::Clip => "clip",
            ElementKind::Text => "text",
            ElementKind::Image => "image",
            ElementKind::InlineImage => "inlineImage",
            ElementKind::Form => "form",
            ElementKind::Shading => "shading",
        }
    }
}

/// Something an operation paints.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub op: u32,
    pub kind: ElementKind,
    pub bbox: Rect,
    /// Outside the clipping path, or invisible text.
    pub hidden: bool,
}

/// What kind of content runs (changes a few checks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Page,
    Form,
    Pattern,
    /// A stream of unknown use (glyph procedure, appearance, ...).
    Other,
}

/// A resource dictionary, with the fonts looked up through it.
pub struct Resources {
    dict: Option<Dict>,
    /// The content has resources (they may still lack an entry). Streams
    /// without `/Resources` may use their parent's, so missing names are
    /// reported more softly.
    pub known: bool,
    fonts: RefCell<HashMap<Vec<u8>, Option<FontRef>>>,
}

pub enum Lookup {
    Found(Option<ObjId>, Object),
    Missing,
}

impl Resources {
    pub fn new(file: &PdfFile, o: Option<&Object>) -> Self {
        let dict = o.and_then(|o| resolve(file, o).1.as_dict().cloned());
        Resources {
            known: dict.is_some(),
            dict,
            fonts: RefCell::default(),
        }
    }

    pub fn lookup(&self, file: &PdfFile, category: &str, name: &Name) -> Lookup {
        let Some(d) = &self.dict else {
            return Lookup::Missing;
        };
        let Some(cat) = d.get_key(category) else {
            return Lookup::Missing;
        };
        let (_, cat) = resolve(file, cat);
        let Some(v) = cat.as_dict().and_then(|c| c.get(name.as_bytes())) else {
            return Lookup::Missing;
        };
        let (id, r) = resolve(file, &v.obj);
        Lookup::Found(id, (*r).clone())
    }
}

/// Where problems, painted elements and drawing go.
pub struct Rec<'d> {
    pub issues: Issues,
    /// Report problems.
    pub report: bool,
    /// Record elements (and follow forms to bound them).
    pub record: bool,
    pub elements: Vec<Element>,
    pub texts: Vec<(u32, String)>,
    /// Some text bounding box used guessed widths.
    pub approximate: bool,
    /// Paint here (forms are followed for it).
    pub device: Option<&'d mut dyn Device>,
}

impl<'d> Rec<'d> {
    /// Report problems and record elements.
    pub fn full() -> Self {
        Rec {
            issues: Issues::default(),
            report: true,
            record: true,
            elements: Vec::new(),
            texts: Vec::new(),
            approximate: false,
            device: None,
        }
    }

    /// Record elements only (inside forms).
    fn elements_only() -> Self {
        Rec {
            report: false,
            ..Self::full()
        }
    }

    /// Only follow the state.
    pub fn silent() -> Self {
        Rec {
            report: false,
            record: false,
            ..Self::full()
        }
    }

    /// Only paint on `device`.
    pub fn draw(device: &'d mut dyn Device) -> Self {
        Rec {
            device: Some(device),
            ..Self::silent()
        }
    }

    fn drawing(&self) -> bool {
        self.device.is_some()
    }

    fn report(
        &mut self,
        op: u32,
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
    ) {
        if self.report {
            self.issues.push(Issue {
                severity,
                code,
                message: message.into(),
                op: Some(op),
                at: None,
            });
        }
    }

    fn error(&mut self, op: u32, code: &'static str, message: impl Into<String>) {
        self.report(op, Severity::Error, code, message);
    }

    fn warning(&mut self, op: u32, code: &'static str, message: impl Into<String>) {
        self.report(op, Severity::Warning, code, message);
    }

    fn info(&mut self, op: u32, code: &'static str, message: impl Into<String>) {
        self.report(op, Severity::Info, code, message);
    }
}

/// Shared by a content and the forms it calls.
pub struct Env<'a> {
    pub file: &'a PdfFile,
    pub cache: &'a DocContent,
    /// Operations left for forms.
    pub budget: u64,
    active: Vec<ObjId>,
    /// Polled while following forms; stops everything when it says so.
    pub cancel: Option<&'a dyn Fn() -> bool>,
}

impl<'a> Env<'a> {
    pub fn new(file: &'a PdfFile, cache: &'a DocContent) -> Self {
        Env {
            file,
            cache,
            budget: FORM_BUDGET,
            active: Vec::new(),
            cancel: None,
        }
    }
}

fn num(n: &Node) -> f64 {
    n.obj.as_f64().unwrap_or(0.0)
}

fn name_text(n: &Name) -> String {
    n.as_str_lossy().into_owned()
}

fn op_name(op: &Op) -> String {
    op.operator.name().into_owned()
}

/// A rectangle as a closed path, transformed by `m`.
fn rect_path(r: &Rect, m: &Matrix) -> impl Iterator<Item = PathEl> {
    let p = |x, y| apply(m, x, y);
    let [a, b, c, d] = [p(r.x0, r.y0), p(r.x1, r.y0), p(r.x1, r.y1), p(r.x0, r.y1)];
    [
        PathEl::Move(a.0, a.1),
        PathEl::Line(b.0, b.1),
        PathEl::Line(c.0, c.1),
        PathEl::Line(d.0, d.1),
        PathEl::Close,
    ]
    .into_iter()
}

/// Whether an operator may appear between `BT` and `ET`.
fn allowed_in_text(c: Category) -> bool {
    matches!(
        c,
        Category::GeneralState
            | Category::Color
            | Category::TextState
            | Category::TextPositioning
            | Category::TextShowing
            | Category::MarkedContent
            | Category::Compatibility
            | Category::Unknown
    )
}

fn matrix_of(nodes: &[Node]) -> Matrix {
    let mut m = IDENTITY;
    for (x, n) in m.iter_mut().zip(nodes) {
        *x = num(n);
    }
    m
}

fn type_ok(a: Arg, o: &Object) -> bool {
    match a {
        Arg::Num | Arg::Int => matches!(o, Object::Integer(_) | Object::Real(_)),
        Arg::Name => matches!(o, Object::Name(_)),
        Arg::Str => matches!(o, Object::String(_)),
        Arg::Array => matches!(o, Object::Array(_)),
        Arg::Props => matches!(o, Object::Name(_) | Object::Dict(_)),
    }
}

/// Number of components and family of a color space object.
pub fn describe_space(file: &PdfFile, o: &Object) -> (String, Option<usize>) {
    let (_, r) = resolve(file, o);
    let (family, args): (String, &[Node]) = match &*r {
        Object::Name(n) => (name_text(n), &[]),
        Object::Array(a) => match a.first().map(|n| resolve(file, &n.obj).1) {
            Some(f) => match f.as_name() {
                Some(n) => (name_text(n), &a[1..]),
                None => return ("?".into(), None),
            },
            None => return ("?".into(), None),
        },
        _ => return ("?".into(), None),
    };
    let n = match family.as_str() {
        "DeviceGray" | "CalGray" | "G" | "Indexed" | "I" | "Separation" => Some(1),
        "DeviceRGB" | "CalRGB" | "RGB" | "Lab" => Some(3),
        "DeviceCMYK" | "CMYK" => Some(4),
        "ICCBased" => args.first().and_then(|s| {
            let (_, s) = resolve(file, &s.obj);
            let d = s.as_dict()?;
            let n = d.get_key("N").map(|n| resolve(file, n).1.as_i64())??;
            usize::try_from(n).ok()
        }),
        "DeviceN" => args
            .first()
            .and_then(|a| resolve(file, &a.obj).1.as_array().map(<[_]>::len)),
        // Uncolored tiling patterns take the components of the base space.
        "Pattern" => match args.first() {
            Some(base) => describe_space(file, &base.obj).1,
            None => Some(0),
        },
        _ => None,
    };
    (family, n)
}

fn initial_color(
    space: String,
    family: String,
    n: Option<usize>,
    def: Option<Arc<Object>>,
) -> Color {
    let components = match family.as_str() {
        "DeviceCMYK" => vec![0.0, 0.0, 0.0, 1.0],
        "Separation" | "DeviceN" => vec![1.0; n.unwrap_or(1)],
        "Pattern" => Vec::new(),
        _ => vec![0.0; n.unwrap_or(0)],
    };
    Color {
        space,
        family,
        n,
        components,
        pattern: None,
        def,
    }
}

/// Inline image abbreviations of color spaces.
fn device_space(n: &[u8], inline: bool) -> Option<&'static str> {
    Some(match n {
        b"DeviceGray" => "DeviceGray",
        b"DeviceRGB" => "DeviceRGB",
        b"DeviceCMYK" => "DeviceCMYK",
        b"G" if inline => "DeviceGray",
        b"RGB" if inline => "DeviceRGB",
        b"CMYK" if inline => "DeviceCMYK",
        _ => return None,
    })
}

enum Piece<'s> {
    Str(&'s [u8]),
    Adjust(f64),
}

pub struct Interp<'r> {
    pub s: State,
    res: &'r Resources,
    kind: Kind,
    /// The decoded streams the operations come from (for inline image
    /// data while drawing).
    data: &'r [SourceData],
}

impl<'r> Interp<'r> {
    pub fn new(s: State, res: &'r Resources, kind: Kind) -> Self {
        Interp {
            s,
            res,
            kind,
            data: &[],
        }
    }

    pub fn with_data(mut self, data: &'r [SourceData]) -> Self {
        self.data = data;
        self
    }

    /// Operands of `op`, checked against its signature. `None` when they
    /// are unusable (too few, wrong types).
    fn operands<'o>(&self, i: u32, op: &'o Op, rec: &mut Rec) -> Option<&'o [Node]> {
        let args = op.operator.args();
        let have = op.operands.len();
        let n = args.len();
        if op.operator.is_variadic() || matches!(op.operator, Operator::Unknown(_)) {
            return Some(&op.operands);
        }
        if have < n {
            rec.error(
                i,
                "Operands",
                format!("{} takes {n} operand(s), {have} given", op_name(op)),
            );
            return None;
        }
        if have > n {
            rec.warning(
                i,
                "Operands",
                format!(
                    "{} takes {n} operand(s), {have} given; the first {} are ignored",
                    op_name(op),
                    have - n
                ),
            );
        }
        let used = &op.operands[have - n..];
        for (k, (a, node)) in args.iter().zip(used).enumerate() {
            if !type_ok(*a, &node.obj) {
                rec.error(
                    i,
                    "OperandType",
                    format!(
                        "operand {} of {} should be a {}, not {}",
                        k + 1,
                        op_name(op),
                        a.describe(),
                        node.obj.type_name()
                    ),
                );
                return None;
            }
            if *a == Arg::Int && matches!(node.obj, Object::Real(r) if r.fract() != 0.0) {
                rec.warning(
                    i,
                    "OperandType",
                    format!("operand {} of {} should be an integer", k + 1, op_name(op)),
                );
            }
        }
        Some(used)
    }

    /// Look up a resource, reporting a missing one.
    fn resource(
        &self,
        env: &Env,
        i: u32,
        rec: &mut Rec,
        category: &str,
        name: &Name,
    ) -> Option<(Option<ObjId>, Object)> {
        match self.res.lookup(env.file, category, name) {
            Lookup::Found(id, o) => {
                if matches!(o, Object::Null) {
                    rec.error(
                        i,
                        "UndefinedResource",
                        format!(
                            "/{category} /{} refers to a missing object",
                            name_text(name)
                        ),
                    );
                    return None;
                }
                Some((id, o))
            }
            Lookup::Missing => {
                if self.res.known {
                    rec.error(
                        i,
                        "UndefinedResource",
                        format!("/{} is not in /Resources /{category}", name_text(name)),
                    );
                } else {
                    rec.info(
                        i,
                        "UndefinedResource",
                        format!(
                            "/{} cannot be looked up: the stream has no /Resources (it may use its parent's)",
                            name_text(name)
                        ),
                    );
                }
                None
            }
        }
    }

    fn font(&self, env: &Env, i: u32, rec: &mut Rec, name: &Name) -> Option<FontRef> {
        if let Some(f) = self.res.fonts.borrow().get(name.as_bytes()) {
            return f.clone();
        }
        let f = self
            .resource(env, i, rec, "Font", name)
            .and_then(|(id, o)| match o.as_dict() {
                Some(d) => Some(FontRef {
                    name: name_text(name),
                    id,
                    info: env.cache.font(env.file, id, d),
                }),
                None => {
                    rec.error(
                        i,
                        "NotAFont",
                        format!("/Font /{} is not a dictionary", name_text(name)),
                    );
                    None
                }
            });
        self.res
            .fonts
            .borrow_mut()
            .insert(name.as_bytes().to_vec(), f.clone());
        f
    }

    fn element(&mut self, i: u32, kind: ElementKind, bbox: Rect, rec: &mut Rec) {
        if !rec.record {
            return;
        }
        let hidden = self
            .s
            .g
            .clip
            .is_some_and(|c| intersect(&c, &bbox).is_none());
        rec.elements.push(Element {
            op: i,
            kind,
            bbox,
            hidden,
        });
    }

    /// Run operation `i`.
    pub fn step(&mut self, env: &mut Env, i: u32, op: &Op, rec: &mut Rec) {
        let cat = op.operator.category();
        self.placement(i, op, cat, rec);
        let Some(args) = self.operands(i, op, rec) else {
            return;
        };
        let g = &mut self.s.g;
        match &op.operator {
            Operator::LineWidth => g.line_width = num(&args[0]),
            Operator::LineCap => {
                g.line_cap = num(&args[0]) as i64;
                if !(0..=2).contains(&g.line_cap) {
                    rec.warning(
                        i,
                        "OperandValue",
                        format!("line cap {} is not 0, 1 or 2", g.line_cap),
                    );
                }
            }
            Operator::LineJoin => {
                g.line_join = num(&args[0]) as i64;
                if !(0..=2).contains(&g.line_join) {
                    rec.warning(
                        i,
                        "OperandValue",
                        format!("line join {} is not 0, 1 or 2", g.line_join),
                    );
                }
            }
            Operator::MiterLimit => g.miter_limit = num(&args[0]),
            Operator::Dash => {
                let a = args[0].obj.as_array().unwrap_or(&[]);
                let dash: Vec<f64> = a.iter().map(num).collect();
                if a.iter().any(|n| n.obj.as_f64().is_none()) || dash.iter().any(|&d| d < 0.0) {
                    rec.error(
                        i,
                        "OperandValue",
                        "dash array entries must be non-negative numbers",
                    );
                } else if !dash.is_empty() && dash.iter().all(|&d| d == 0.0) {
                    rec.warning(i, "OperandValue", "dash array entries are all zero");
                }
                g.dash = dash;
                g.dash_phase = num(&args[1]);
            }
            Operator::Intent => {
                let n = args[0].obj.as_name().map(name_text).unwrap_or_default();
                if !matches!(
                    n.as_str(),
                    "AbsoluteColorimetric" | "RelativeColorimetric" | "Saturation" | "Perceptual"
                ) {
                    rec.info(i, "OperandValue", format!("unknown rendering intent /{n}"));
                }
                g.intent = n;
            }
            Operator::Flatness => g.flatness = num(&args[0]),
            Operator::ExtGState => {
                if let Some(n) = args[0].obj.as_name() {
                    if let Some((_, o)) = self.resource(env, i, rec, "ExtGState", n) {
                        match o.as_dict() {
                            Some(d) => self.ext_gstate(env, i, &name_text(n), d, rec),
                            None => rec.error(
                                i,
                                "NotADictionary",
                                format!("/ExtGState /{} is not a dictionary", name_text(n)),
                            ),
                        }
                    }
                }
            }
            Operator::Save => {
                if self.s.stack.len() < MAX_STACK {
                    self.s.stack.push(self.s.g.clone());
                } else {
                    self.s.stack_overflow += 1;
                }
            }
            Operator::Restore => {
                if self.s.stack_overflow > 0 {
                    self.s.stack_overflow -= 1;
                } else if let Some(g) = self.s.stack.pop() {
                    self.s.g = g;
                } else {
                    rec.error(i, "Unbalanced", "Q without a matching q");
                }
            }
            Operator::Concat => {
                let m = matrix_of(args);
                if m[0] * m[3] - m[1] * m[2] == 0.0 {
                    rec.warning(
                        i,
                        "OperandValue",
                        "the matrix is singular (nothing drawn after it is visible)",
                    );
                }
                g.ctm = mul(&m, &g.ctm);
            }
            Operator::MoveTo => {
                let (x, y) = (num(&args[0]), num(&args[1]));
                let ctm = g.ctm;
                let p = self.s.path.get_or_insert_with(PathState::default);
                let (dx, dy) = apply(&ctm, x, y);
                add_point(&mut p.bbox, dx, dy);
                if rec.drawing() {
                    p.els.push(PathEl::Move(dx, dy));
                }
                p.current = Some((x, y));
                p.start = Some((x, y));
                p.segments += 1;
            }
            Operator::LineTo | Operator::CurveTo | Operator::CurveV | Operator::CurveY => {
                let ctm = g.ctm;
                let pts: Vec<(f64, f64)> =
                    args.chunks(2).map(|c| (num(&c[0]), num(&c[1]))).collect();
                match self.s.path.as_mut().filter(|p| p.current.is_some()) {
                    Some(p) => {
                        for &(x, y) in &pts {
                            let (dx, dy) = apply(&ctm, x, y);
                            add_point(&mut p.bbox, dx, dy);
                        }
                        if rec.drawing() {
                            let d = |(x, y)| apply(&ctm, x, y);
                            let cur = p.current.unwrap_or_default();
                            let (a, b, c) = match (&op.operator, pts.as_slice()) {
                                (Operator::CurveTo, &[a, b, c]) => (a, b, c),
                                (Operator::CurveV, &[b, c]) => (cur, b, c),
                                (Operator::CurveY, &[a, c]) => (a, c, c),
                                (_, &[.., c]) => (c, c, c),
                                _ => (cur, cur, cur),
                            };
                            p.els.push(if op.operator == Operator::LineTo {
                                let (x, y) = d(c);
                                PathEl::Line(x, y)
                            } else {
                                let ((x1, y1), (x2, y2), (x3, y3)) = (d(a), d(b), d(c));
                                PathEl::Curve(x1, y1, x2, y2, x3, y3)
                            });
                        }
                        p.current = pts.last().copied();
                        p.segments += 1;
                    }
                    None => rec.error(
                        i,
                        "NoCurrentPoint",
                        format!("{} without a current point (m or re first)", op_name(op)),
                    ),
                }
            }
            Operator::ClosePath => match self.s.path.as_mut() {
                Some(p) if p.current.is_some() => {
                    p.current = p.start;
                    if rec.drawing() {
                        p.els.push(PathEl::Close);
                    }
                }
                _ => rec.warning(i, "NoCurrentPoint", "h without a current point"),
            },
            Operator::Rect => {
                let (x, y, w, h) = (num(&args[0]), num(&args[1]), num(&args[2]), num(&args[3]));
                let ctm = g.ctm;
                let p = self.s.path.get_or_insert_with(PathState::default);
                for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                    let (dx, dy) = apply(&ctm, px, py);
                    add_point(&mut p.bbox, dx, dy);
                }
                if rec.drawing() {
                    p.els.extend(rect_path(
                        &Rect {
                            x0: x,
                            y0: y,
                            x1: x + w,
                            y1: y + h,
                        },
                        &ctm,
                    ));
                }
                p.current = Some((x, y));
                p.start = Some((x, y));
                p.segments += 1;
            }
            Operator::Stroke | Operator::CloseStroke => {
                self.paint(i, op, Some(ElementKind::Stroke), rec)
            }
            Operator::Fill | Operator::FillObsolete | Operator::FillEvenOdd => {
                self.paint(i, op, Some(ElementKind::Fill), rec)
            }
            Operator::FillStroke
            | Operator::FillStrokeEvenOdd
            | Operator::CloseFillStroke
            | Operator::CloseFillStrokeEvenOdd => {
                self.paint(i, op, Some(ElementKind::FillStroke), rec)
            }
            Operator::EndPath => self.paint(i, op, None, rec),
            Operator::Clip | Operator::ClipEvenOdd => {
                if self.s.path.is_none() {
                    rec.warning(
                        i,
                        "NoPath",
                        format!("{} without a current path", op_name(op)),
                    );
                }
                self.s.pending_clip = Some(op.operator == Operator::ClipEvenOdd);
            }
            Operator::BeginText => {
                if self.s.in_text {
                    rec.error(
                        i,
                        "Nesting",
                        "BT inside a text object (text objects do not nest)",
                    );
                }
                self.s.in_text = true;
                self.s.tm = IDENTITY;
                self.s.tlm = IDENTITY;
            }
            Operator::EndText => {
                if !self.s.in_text {
                    rec.error(i, "Unbalanced", "ET without a matching BT");
                }
                self.s.in_text = false;
            }
            Operator::CharSpacing => g.text.char_spacing = num(&args[0]),
            Operator::WordSpacing => g.text.word_spacing = num(&args[0]),
            Operator::HorizontalScale => g.text.scale = num(&args[0]),
            Operator::Leading => g.text.leading = num(&args[0]),
            Operator::Font => {
                let size = num(&args[1]);
                if let Some(n) = args[0].obj.as_name() {
                    let f = self.font(env, i, rec, n);
                    self.s.g.text.font = f;
                }
                self.s.g.text.size = size;
            }
            Operator::RenderMode => {
                g.text.render = num(&args[0]) as i64;
                if !(0..=7).contains(&g.text.render) {
                    rec.warning(
                        i,
                        "OperandValue",
                        format!("text rendering mode {} is not 0–7", g.text.render),
                    );
                }
            }
            Operator::Rise => g.text.rise = num(&args[0]),
            Operator::TextMove => self.text_move(num(&args[0]), num(&args[1])),
            Operator::TextMoveLeading => {
                let ty = num(&args[1]);
                self.s.g.text.leading = -ty;
                self.text_move(num(&args[0]), ty);
            }
            Operator::TextMatrix => {
                let m = matrix_of(args);
                self.s.tm = m;
                self.s.tlm = m;
            }
            Operator::NextLine => {
                let l = self.s.g.text.leading;
                self.text_move(0.0, -l);
            }
            Operator::ShowText => {
                if let Some(s) = args[0].obj.as_string() {
                    self.show(i, &[Piece::Str(&s.bytes)], rec);
                }
            }
            Operator::NextLineShow => {
                let l = self.s.g.text.leading;
                self.text_move(0.0, -l);
                if let Some(s) = args[0].obj.as_string() {
                    self.show(i, &[Piece::Str(&s.bytes)], rec);
                }
            }
            Operator::NextLineSpacingShow => {
                self.s.g.text.word_spacing = num(&args[0]);
                self.s.g.text.char_spacing = num(&args[1]);
                let l = self.s.g.text.leading;
                self.text_move(0.0, -l);
                if let Some(s) = args[2].obj.as_string() {
                    self.show(i, &[Piece::Str(&s.bytes)], rec);
                }
            }
            Operator::ShowTextArray => {
                let a = args[0].obj.as_array().unwrap_or(&[]);
                let mut pieces = Vec::with_capacity(a.len());
                let mut bad = false;
                for n in a {
                    match &n.obj {
                        Object::String(s) => pieces.push(Piece::Str(&s.bytes)),
                        Object::Integer(_) | Object::Real(_) => pieces.push(Piece::Adjust(num(n))),
                        _ => bad = true,
                    }
                }
                if bad {
                    rec.error(
                        i,
                        "OperandType",
                        "TJ array elements must be strings and numbers",
                    );
                }
                self.show(i, &pieces, rec);
            }
            Operator::GlyphWidth | Operator::GlyphWidthBox => {
                if self.kind == Kind::Page || i != 0 {
                    rec.warning(
                        i,
                        "Placement",
                        format!(
                            "{} is only valid as the first operator of a Type 3 glyph description",
                            op_name(op)
                        ),
                    );
                }
            }
            Operator::StrokeSpace | Operator::FillSpace => {
                if let Some(n) = args[0].obj.as_name() {
                    let c = self.color_space(env, i, rec, n);
                    if op.operator == Operator::StrokeSpace {
                        self.s.g.stroke = c;
                    } else {
                        self.s.g.fill = c;
                    }
                }
            }
            Operator::StrokeColor
            | Operator::StrokeColorN
            | Operator::FillColor
            | Operator::FillColorN => {
                self.set_color(env, i, op, rec);
            }
            Operator::StrokeGray => {
                g.stroke = Color::device("DeviceGray", args.iter().map(num).collect())
            }
            Operator::FillGray => {
                g.fill = Color::device("DeviceGray", args.iter().map(num).collect())
            }
            Operator::StrokeRgb => {
                g.stroke = Color::device("DeviceRGB", args.iter().map(num).collect())
            }
            Operator::FillRgb => {
                g.fill = Color::device("DeviceRGB", args.iter().map(num).collect())
            }
            Operator::StrokeCmyk => {
                g.stroke = Color::device("DeviceCMYK", args.iter().map(num).collect())
            }
            Operator::FillCmyk => {
                g.fill = Color::device("DeviceCMYK", args.iter().map(num).collect())
            }
            Operator::Shade => {
                if let Some(n) = args[0].obj.as_name() {
                    if let Some((_, o)) = self.resource(env, i, rec, "Shading", n) {
                        if let Some(c) = self.s.g.clip {
                            self.element(i, ElementKind::Shading, c, rec);
                        }
                        if let Some(d) = rec.device.as_deref_mut() {
                            d.shading(&o, &self.s.g);
                        }
                    }
                }
            }
            Operator::InlineImage => self.inline_image(env, i, op, rec),
            Operator::ImageData | Operator::EndImage => {
                rec.error(
                    i,
                    "InlineImage",
                    format!("{} outside an inline image", op_name(op)),
                );
            }
            Operator::XObject => {
                if let Some(n) = args[0].obj.as_name() {
                    self.xobject(env, i, n, rec);
                }
            }
            Operator::MarkPoint => {}
            Operator::MarkPointProps | Operator::BeginMarkedProps => {
                if let Some(n) = args[1].obj.as_name() {
                    self.resource(env, i, rec, "Properties", n);
                }
                if op.operator == Operator::BeginMarkedProps {
                    let tag = args[0].obj.as_name().map(name_text).unwrap_or_default();
                    self.s.marked.push(tag);
                }
            }
            Operator::BeginMarked => {
                let tag = args[0].obj.as_name().map(name_text).unwrap_or_default();
                self.s.marked.push(tag);
            }
            Operator::EndMarked => {
                if self.s.marked.pop().is_none() {
                    rec.error(i, "Unbalanced", "EMC without a matching BMC or BDC");
                }
            }
            Operator::BeginCompat => self.s.compat += 1,
            Operator::EndCompat => {
                if self.s.compat == 0 {
                    rec.error(i, "Unbalanced", "EX without a matching BX");
                } else {
                    self.s.compat -= 1;
                }
            }
            Operator::Unknown(_) => {
                if self.s.compat > 0 {
                    rec.info(
                        i,
                        "UnknownOperator",
                        format!("unknown operator {} (ignored inside BX … EX)", op_name(op)),
                    );
                } else {
                    rec.error(
                        i,
                        "UnknownOperator",
                        format!("unknown operator {}", op_name(op)),
                    );
                }
            }
        }
    }

    /// Checks of where an operator appears: in or out of a text object,
    /// inside a path object, after `W`.
    fn placement(&mut self, i: u32, op: &Op, cat: Category, rec: &mut Rec) {
        let painting = matches!(cat, Category::PathPainting | Category::Clipping);
        if self.s.pending_clip.is_some() && !painting {
            rec.warning(
                i,
                "Placement",
                "W / W* is not followed by a path-painting operator; the clip is ignored",
            );
            self.s.pending_clip = None;
        }
        if self.s.path.is_some() && !painting && cat != Category::PathConstruction {
            rec.warning(
                i,
                "Placement",
                format!(
                    "{} inside a path object (between m / re and the painting operator)",
                    op_name(op)
                ),
            );
        }
        if self.s.in_text {
            if !allowed_in_text(cat) && cat != Category::TextObject {
                rec.warning(
                    i,
                    "Placement",
                    format!("{} inside a text object (BT … ET)", op_name(op)),
                );
            }
        } else if matches!(cat, Category::TextPositioning | Category::TextShowing) {
            rec.error(
                i,
                "Placement",
                format!("{} outside a text object (BT … ET)", op_name(op)),
            );
        }
    }

    fn paint(&mut self, i: u32, op: &Op, kind: Option<ElementKind>, rec: &mut Rec) {
        let mut path = self.s.path.take();
        let clip = self.s.pending_clip.take();
        let Some(bbox) = path.as_ref().and_then(|p| p.bbox) else {
            if kind.is_some() {
                rec.warning(
                    i,
                    "NoPath",
                    format!("{} without a current path: nothing is painted", op_name(op)),
                );
            }
            return;
        };
        let stroked = matches!(kind, Some(ElementKind::Stroke | ElementKind::FillStroke));
        let painted = if stroked {
            grow(
                &bbox,
                self.s.g.line_width.abs() * scale_of(&self.s.g.ctm) / 2.0,
            )
        } else {
            bbox
        };
        match kind {
            Some(k) => self.element(i, k, painted, rec),
            None if clip.is_some() => self.element(i, ElementKind::Clip, bbox, rec),
            None => {}
        }
        if let (Some(d), Some(p)) = (rec.device.as_deref_mut(), path.as_mut()) {
            if matches!(
                op.operator,
                Operator::CloseStroke
                    | Operator::CloseFillStroke
                    | Operator::CloseFillStrokeEvenOdd
            ) {
                p.els.push(PathEl::Close);
            }
            let even_odd = matches!(
                op.operator,
                Operator::FillEvenOdd
                    | Operator::FillStrokeEvenOdd
                    | Operator::CloseFillStrokeEvenOdd
            );
            let g = &self.s.g;
            if matches!(kind, Some(ElementKind::Fill | ElementKind::FillStroke)) {
                d.fill(&p.els, even_odd, g);
            }
            if stroked {
                d.stroke(&p.els, g);
            }
            if let Some(eo) = clip {
                let g = &mut self.s.g;
                g.clip_path = Some(Arc::new(ClipPath {
                    path: std::mem::take(&mut p.els),
                    even_odd: eo,
                    parent: g.clip_path.take(),
                }));
            }
        }
        // The clip applies after painting.
        if clip.is_some() {
            let g = &mut self.s.g;
            g.clip = Some(match g.clip {
                None => bbox,
                Some(c) => intersect(&c, &bbox).unwrap_or(Rect {
                    x0: c.x0,
                    y0: c.y0,
                    x1: c.x0,
                    y1: c.y0,
                }),
            });
            if g.clip.is_some_and(|c| {
                c.x0 == c.x1 && c.y0 == c.y1 && (bbox.x0 != bbox.x1 || bbox.y0 != bbox.y1)
            }) {
                rec.info(i, "EmptyClip", "the clipping path does not overlap the current one: nothing after it is visible");
            }
        }
    }

    fn text_move(&mut self, tx: f64, ty: f64) {
        self.s.tlm = mul(&translate(tx, ty), &self.s.tlm);
        self.s.tm = self.s.tlm;
    }

    /// Show strings (with `TJ` adjustments), advancing the text matrix.
    fn show(&mut self, i: u32, pieces: &[Piece], rec: &mut Rec) {
        let t = self.s.g.text.clone();
        let Some(font) = t.font.as_ref() else {
            rec.error(i, "NoFont", "text shown before a font is set (Tf)");
            return;
        };
        let f = &font.info;
        let th = t.scale / 100.0;
        let size = t.size;
        let (mut x, mut y) = (0.0f64, 0.0f64);
        let mut bbox: Option<Rect> = None;
        let mut text = String::new();
        let mut decoded = false;
        let drawing = rec.drawing();
        let mut glyphs = Vec::new();
        for p in pieces {
            match p {
                Piece::Adjust(n) => {
                    if f.vertical {
                        y -= n / 1000.0 * size;
                    } else {
                        x -= n / 1000.0 * size * th;
                    }
                    if *n <= SPACE_ADJUST && !text.is_empty() && !text.ends_with(' ') {
                        text.push(' ');
                    }
                }
                Piece::Str(s) => {
                    for c in f.codes(s) {
                        let w0 = f.advance(c);
                        let spacing =
                            t.char_spacing + if f.is_space(c) { t.word_spacing } else { 0.0 };
                        let (ox, oy) = (x, y);
                        let glyph = if f.vertical {
                            let ty = w0 * size + spacing;
                            let r = Rect {
                                x0: -0.5 * size,
                                y0: y + ty.min(0.0),
                                x1: 0.5 * size,
                                y1: y + ty.max(0.0),
                            };
                            y += ty;
                            r
                        } else {
                            let w = w0 * size * th;
                            let r = Rect {
                                x0: x.min(x + w),
                                y0: t.rise + f.descent * size,
                                x1: x.max(x + w),
                                y1: t.rise + f.ascent * size,
                            };
                            x += (w0 * size + spacing) * th;
                            r
                        };
                        bbox = Some(union(bbox, glyph));
                        if drawing {
                            glyphs.push(Glyph {
                                code: c.code,
                                x: ox,
                                y: oy + t.rise,
                                bbox: glyph,
                            });
                        }
                        match f.text(c) {
                            Some(s) => {
                                decoded = true;
                                text.push_str(&s);
                            }
                            None => text.push('\u{fffd}'),
                        }
                    }
                }
            }
        }
        if rec.record {
            if let Some(b) = bbox {
                let m = mul(&self.s.tm, &self.s.g.ctm);
                let hidden = t.render == 3 || t.render == 7;
                self.element(i, ElementKind::Text, transform(&b, &m), rec);
                if hidden {
                    if let Some(e) = rec.elements.last_mut() {
                        e.hidden = true;
                    }
                }
                rec.approximate |= f.approximate;
            }
            if decoded {
                rec.texts.push((i, text));
            }
        }
        if let Some(d) = rec.device.as_deref_mut() {
            if !glyphs.is_empty() && t.render != 3 && t.render != 7 {
                d.text(
                    &GlyphRun {
                        font,
                        matrix: mul(&self.s.tm, &self.s.g.ctm),
                        size,
                        scale: th,
                        rise: t.rise,
                        glyphs,
                    },
                    &self.s.g,
                );
            }
        }
        self.s.tm = mul(&translate(x, y), &self.s.tm);
    }

    fn color_space(&mut self, env: &Env, i: u32, rec: &mut Rec, n: &Name) -> Color {
        if let Some(d) = device_space(n.as_bytes(), false) {
            return initial_color(
                d.into(),
                d.into(),
                Some(if d == "DeviceCMYK" {
                    4
                } else if d == "DeviceRGB" {
                    3
                } else {
                    1
                }),
                None,
            );
        }
        if n == "Pattern" {
            return initial_color("Pattern".into(), "Pattern".into(), Some(0), None);
        }
        if device_space(n.as_bytes(), true).is_some() || n == "I" {
            rec.warning(
                i,
                "OperandValue",
                format!(
                    "/{} is an inline image abbreviation; use the full name",
                    name_text(n)
                ),
            );
        }
        match self.resource(env, i, rec, "ColorSpace", n) {
            Some((_, o)) => {
                let (family, comps) = describe_space(env.file, &o);
                initial_color(name_text(n), family, comps, Some(Arc::new(o)))
            }
            None => initial_color(name_text(n), "?".into(), None, None),
        }
    }

    fn set_color(&mut self, env: &Env, i: u32, op: &Op, rec: &mut Rec) {
        let stroke = matches!(op.operator, Operator::StrokeColor | Operator::StrokeColorN);
        let with_name = matches!(op.operator, Operator::StrokeColorN | Operator::FillColorN);
        let mut operands: &[Node] = &op.operands;
        let mut pattern = None;
        if let Some(Object::Name(n)) = operands.last().map(|n| &n.obj) {
            if with_name {
                pattern = Some(n.clone());
            } else {
                rec.error(
                    i,
                    "OperandType",
                    format!(
                        "{} takes numbers only (use {}N for patterns)",
                        op_name(op),
                        op_name(op)
                    ),
                );
            }
            operands = &operands[..operands.len() - 1];
        }
        if operands.iter().any(|n| n.obj.as_f64().is_none()) {
            rec.error(
                i,
                "OperandType",
                format!(
                    "{} takes numbers{}",
                    op_name(op),
                    if with_name { " and a pattern name" } else { "" }
                ),
            );
            return;
        }
        let cur = if stroke {
            &self.s.g.stroke
        } else {
            &self.s.g.fill
        };
        let family = cur.family.clone();
        if pattern.is_some() && family != "Pattern" {
            rec.warning(
                i,
                "ColorSpace",
                format!("pattern name given, but the color space is {}", cur.space),
            );
        }
        if pattern.is_none() && family == "Pattern" {
            rec.error(
                i,
                "ColorSpace",
                "the color space is Pattern: a pattern name is needed",
            );
        }
        if !with_name
            && !matches!(
                family.as_str(),
                "DeviceGray"
                    | "DeviceRGB"
                    | "DeviceCMYK"
                    | "CalGray"
                    | "CalRGB"
                    | "Lab"
                    | "Indexed"
            )
        {
            rec.info(
                i,
                "ColorSpace",
                format!(
                    "{} is meant for device, CIE-based and Indexed color spaces; with {} use {}N",
                    op_name(op),
                    family,
                    op_name(op)
                ),
            );
        }
        // Without a pattern name the count is meaningless (reported above).
        if let Some(n) = cur.n.filter(|_| family != "Pattern" || pattern.is_some()) {
            if operands.len() != n {
                rec.warning(
                    i,
                    "ColorComponents",
                    format!(
                        "{} gives {} component(s); {} has {n}",
                        op_name(op),
                        operands.len(),
                        cur.space
                    ),
                );
            }
        }
        if let Some(p) = &pattern {
            self.resource(env, i, rec, "Pattern", p);
        }
        let c = if stroke {
            &mut self.s.g.stroke
        } else {
            &mut self.s.g.fill
        };
        c.components = operands.iter().map(num).collect();
        c.pattern = pattern.as_ref().map(name_text);
    }

    fn ext_gstate(&mut self, env: &Env, i: u32, name: &str, d: &Dict, rec: &mut Rec) {
        let f = env.file;
        let g = &mut self.s.g;
        g.ext_gstates.push(name.into());
        for (k, v) in d {
            let (_, v) = resolve(f, &v.obj);
            match k.as_bytes() {
                b"LW" => g.line_width = v.as_f64().unwrap_or(g.line_width),
                b"LC" => g.line_cap = v.as_i64().unwrap_or(g.line_cap),
                b"LJ" => g.line_join = v.as_i64().unwrap_or(g.line_join),
                b"ML" => g.miter_limit = v.as_f64().unwrap_or(g.miter_limit),
                b"D" => {
                    if let Some([a, p]) = v.as_array() {
                        g.dash = a.obj.as_array().unwrap_or(&[]).iter().map(num).collect();
                        g.dash_phase = num(p);
                    }
                }
                b"RI" => {
                    if let Some(n) = v.as_name() {
                        g.intent = name_text(n);
                    }
                }
                b"FL" => g.flatness = v.as_f64().unwrap_or(g.flatness),
                b"CA" => g.stroke_alpha = v.as_f64().unwrap_or(g.stroke_alpha),
                b"ca" => g.fill_alpha = v.as_f64().unwrap_or(g.fill_alpha),
                b"BM" => {
                    let n = v.as_name().or_else(|| v.as_array()?.first()?.obj.as_name());
                    if let Some(n) = n {
                        g.blend = name_text(n);
                    }
                }
                b"SMask" => {
                    g.soft_mask = match &*v {
                        Object::Name(n) if n == "None" => None,
                        Object::Dict(sd) => Some(
                            sd.get_name("S")
                                .map_or("mask".into(), |s| format!("/{} mask", name_text(s))),
                        ),
                        _ => Some("mask".into()),
                    }
                }
                b"Font" => {
                    if let Some([fo, size]) = v.as_array() {
                        let (id, fr) = resolve(f, &fo.obj);
                        match fr.as_dict() {
                            Some(fd) => {
                                g.text.font = Some(FontRef {
                                    name: "ExtGState".into(),
                                    id,
                                    info: env.cache.font(f, id, fd),
                                });
                                g.text.size = num(size);
                            }
                            None => rec.error(
                                i,
                                "NotAFont",
                                format!("/Font of /ExtGState /{name} is not a font dictionary"),
                            ),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn inline_image(&mut self, env: &Env, i: u32, op: &Op, rec: &mut Rec) {
        let Some(img) = op.inline_image.as_ref() else {
            return;
        };
        let d = &img.dict;
        let get = |long: &str, short: &str| d.get_key(long).or_else(|| d.get_key(short));
        let mask = matches!(get("ImageMask", "IM"), Some(Object::Bool(true)));
        for (long, short, needed) in [
            ("Width", "W", true),
            ("Height", "H", true),
            ("BitsPerComponent", "BPC", !mask),
            ("ColorSpace", "CS", !mask),
        ] {
            if needed && get(long, short).is_none() {
                rec.error(
                    i,
                    "InlineImage",
                    format!("inline image without /{short} (/{long})"),
                );
            }
        }
        if let Some(Object::Name(n)) = get("ColorSpace", "CS") {
            if device_space(n.as_bytes(), true).is_none() && n != "I" && n != "Indexed" {
                self.resource(env, i, rec, "ColorSpace", n);
            }
        }
        self.element(
            i,
            ElementKind::InlineImage,
            transform(&unit_square(), &self.s.g.ctm),
            rec,
        );
        if let Some(dev) = rec.device.as_deref_mut() {
            let data = op.origin.and_then(|o| {
                let s = self.data.get(o.source as usize)?;
                s.data.get(img.data.start as usize..img.data.end as usize)
            });
            if let Some(data) = data {
                dev.image(ImageData::Inline { dict: d, data }, self.res, &self.s.g);
            }
        }
    }

    fn xobject(&mut self, env: &mut Env, i: u32, n: &Name, rec: &mut Rec) {
        let Some((id, o)) = self.resource(env, i, rec, "XObject", n) else {
            return;
        };
        let Some(s) = o.as_stream() else {
            rec.error(
                i,
                "NotAStream",
                format!("/XObject /{} is not a stream", name_text(n)),
            );
            return;
        };
        match s.dict.get_name("Subtype").map(Name::as_bytes) {
            Some(b"Image") => {
                self.element(
                    i,
                    ElementKind::Image,
                    transform(&unit_square(), &self.s.g.ctm),
                    rec,
                );
                if let Some(d) = rec.device.as_deref_mut() {
                    d.image(ImageData::XObject { id, stream: s }, self.res, &self.s.g);
                }
            }
            Some(b"Form") => {
                if let Some(b) = self.form(env, i, id, s, rec) {
                    self.element(i, ElementKind::Form, b, rec);
                }
            }
            Some(b"PS") => rec.info(
                i,
                "PostScript",
                "PostScript XObjects are ignored by PDF processors",
            ),
            Some(other) => rec.error(
                i,
                "XObjectType",
                format!(
                    "unknown XObject /Subtype /{}",
                    String::from_utf8_lossy(other)
                ),
            ),
            None => rec.error(
                i,
                "XObjectType",
                format!("/XObject /{} has no /Subtype", name_text(n)),
            ),
        }
    }

    /// Follow a form XObject; returns the bounding box of what it paints.
    fn form(
        &mut self,
        env: &mut Env,
        i: u32,
        id: Option<ObjId>,
        s: &Stream,
        rec: &mut Rec,
    ) -> Option<Rect> {
        if !rec.record && !rec.drawing() {
            return None;
        }
        let f = env.file;
        let matrix = s
            .dict
            .get_key("Matrix")
            .map(|m| resolve(f, m).1)
            .and_then(|m| m.as_array().filter(|a| a.len() == 6).map(matrix_of))
            .unwrap_or(IDENTITY);
        let ctm = mul(&matrix, &self.s.g.ctm);
        let bbox = s.dict.get_key("BBox").and_then(|b| Rect::read(f, b));
        if bbox.is_none() {
            rec.error(i, "FormBBox", "form XObject without a valid /BBox");
        }
        let bbox_t = bbox.map(|b| transform(&b, &ctm));
        let Some(id) = id else { return bbox_t };
        if env.active.contains(&id) {
            rec.error(i, "RecursiveForm", format!("form {id} paints itself"));
            return bbox_t;
        }
        if env.active.len() as u32 >= MAX_FORM_DEPTH || env.budget == 0 {
            return bbox_t;
        }
        let data = env.cache.form_data(f, id, s);
        let res = match s.dict.get_key("Resources") {
            Some(r) => Resources::new(f, Some(r)),
            // Resources inherited from the page (allowed before PDF 1.2).
            None => Resources {
                dict: self.res.dict.clone(),
                known: self.res.known,
                fonts: RefCell::default(),
            },
        };
        let mut g = self.s.g.clone();
        g.ctm = ctm;
        if let Some(b) = bbox_t {
            g.clip = Some(match g.clip {
                Some(c) => intersect(&c, &b).unwrap_or(Rect {
                    x0: c.x0,
                    y0: c.y0,
                    x1: c.x0,
                    y1: c.y0,
                }),
                None => b,
            });
            if rec.drawing() {
                g.clip_path = Some(Arc::new(ClipPath {
                    path: rect_path(&b, &IDENTITY).collect(),
                    even_odd: false,
                    parent: g.clip_path.take(),
                }));
            }
        }
        let mut sub = Interp::new(State::new(g), &res, Kind::Form).with_data(&data);
        let mut srec = Rec {
            device: rec.device.as_deref_mut().map(|d| d as &mut dyn Device),
            ..if rec.record {
                Rec::elements_only()
            } else {
                Rec::silent()
            }
        };
        env.active.push(id);
        let mut reader = OpReader::new(&data);
        let mut sink = Issues::default();
        while env.budget > 0 {
            let k = reader.count() as u32;
            if k % 256 == 0 && env.cancel.is_some_and(|c| c()) {
                env.budget = 0;
                break;
            }
            let Some(op) = reader.next(&mut sink) else {
                break;
            };
            env.budget -= 1;
            sub.step(env, k, &op, &mut srec);
        }
        env.active.pop();
        let approximate = srec.approximate;
        let inner = srec
            .elements
            .iter()
            .filter(|e| !e.hidden && e.kind != ElementKind::Clip)
            .fold(None, |a, e| Some(union(a, e.bbox)));
        drop(srec);
        if env.budget == 0 {
            rec.info(
                i,
                "FormBudget",
                format!("stopped following form XObjects after {FORM_BUDGET} operations"),
            );
        }
        rec.approximate |= approximate;
        match (inner, bbox_t) {
            (Some(a), Some(b)) => intersect(&a, &b),
            (a, b) => a.or(b),
        }
    }

    /// Problems left open at the end of the content (content-wide).
    pub fn finish(&self, issues: &mut Issues) {
        let s = &self.s;
        let open = s.stack.len() + s.stack_overflow as usize;
        let mut w = |code, m: String| issues.push(Issue::warning(code, m, None));
        if open > 0 {
            w("Unbalanced", format!("{open} q without Q at the end"));
        }
        if s.in_text {
            w("Unbalanced", "BT without ET at the end".into());
        }
        if !s.marked.is_empty() {
            w(
                "Unbalanced",
                format!("{} BMC / BDC without EMC at the end", s.marked.len()),
            );
        }
        if s.compat > 0 {
            w("Unbalanced", "BX without EX at the end".into());
        }
        if s.path.is_some() {
            w("NoPaint", "a path is built but never painted".into());
        }
    }
}
