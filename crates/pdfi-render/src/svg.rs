//! SVG export: what a content paints (all of it, or what a region of it
//! takes), as SVG to edit in a vector graphics program.
//!
//! Paths keep their geometry (axis-aligned rectangles become `<rect>`),
//! with fill rules, strokes (width, caps, joins, miter limit, dashes) and
//! opacities; strokes under a skewing or unevenly scaling matrix keep it as
//! a `transform`. Text is either `<text>` (font names from `/BaseFont`,
//! weight and style from them, every character placed where it paints;
//! runs of a text object on one line join) or glyph outlines grouped per
//! text-showing operation; glyphs without Unicode text are always
//! outlines. Images are PNG (JPEG files stored as viewers show them are
//! kept as they are), stencil masks in their fill color. Axial and radial
//! shadings become gradients, other shadings images. Tiling patterns
//! become `<pattern>`s, soft masks `<mask>`s.
//!
//! What the file groups stays grouped: saved states (`q … Q`) holding more
//! than one element, text objects, marked content (optional content, the
//! layers of drawing programs, as named groups: Inkscape layers at the top
//! level, hidden when off by default), form XObjects and transparency
//! groups. A clipping path becomes a `<g clip-path>` over what it clips (on
//! the saved state's group when everything in it is clipped alike);
//! rectangles clipping nothing away are left out.
//!
//! Coordinates are the content's view (points from the top left, as
//! [`view`] puts it), transforms baked into them where they can be.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use pdfi_content::device::{ClipPath, Device, Glyph, GlyphRun, Group, ImageData, PathEl, SoftMask};
use pdfi_content::extract::Select;
use pdfi_content::font::Font;
use pdfi_content::geom::{
    IDENTITY, Matrix, apply, grow, intersect, mul, scale_of, transform, union, unit_square,
};
use pdfi_content::interp::{Color, GState, Kind, Resources};
use pdfi_content::{Analysis, Content};
use pdfi_model::{Rect, resolve};
use pdfi_syntax::{DictExt, Name, ObjId, Object, PdfFile, Stream};
use tiny_skia::Pixmap;

use crate::color::{self, Approx, Space};
use crate::image::{self, Decoded, StoredFile};
use crate::shading::{Gradient, GradientKind, Shading};
use crate::{DocRender, view};

/// Patterns and soft masks inside each other.
const MAX_DEPTH: u32 = 8;
/// Slack when testing that a box is inside the region, in points.
const EPSILON: f64 = 1e-3;
/// Slack when testing that a clipping rectangle holds what it clips.
const CLIP_SLACK: f64 = 0.01;
/// Pixels per point of shadings drawn as images.
const SHADING_SCALE: f64 = 2.0;
/// Most pixels of a shading drawn as an image.
const SHADING_PIXELS: f64 = (1 << 22) as f64;
/// Stroke width (points) of hairlines (width 0).
const HAIRLINE: f64 = 0.25;
/// Intervals of a soft mask's transfer function sampled into a table.
const TRANSFER_SAMPLES: usize = 32;
/// Largest JPEG copied as it is stored.
const MAX_JPEG: usize = 256 << 20;

/// How text is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAs {
    /// `<text>` (glyphs without Unicode text as outlines).
    Text,
    /// Glyph outlines.
    Paths,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub text: TextAs,
    /// Only what paints in this rectangle of the content's space, picked
    /// as a region cut out picks it.
    pub region: Option<(Rect, Select)>,
    /// Shrink the view to what is kept (within the region).
    pub fit: bool,
}

/// An SVG file, and what went into it.
#[derive(Debug)]
pub struct Svg {
    pub data: Vec<u8>,
    /// The view box, in the view's points (from the top left).
    pub view_box: Rect,
    /// Elements written (paths, texts, images, shadings) and images.
    pub elements: u64,
    pub images: u64,
    /// What was approximated or left out, with how often.
    pub notes: Vec<(String, u32)>,
}

/// Export an analyzed content as SVG.
pub fn export(
    cache: &DocRender,
    content: &Content<'_>,
    a: &Analysis,
    o: &Options,
    cancel: &dyn Fn() -> bool,
) -> Result<Svg, String> {
    let v = view(a);
    let region = o.region.map(|(r, s)| {
        (
            Rect {
                x0: r.x0.min(r.x1),
                y0: r.y0.min(r.y1),
                x1: r.x0.max(r.x1),
                y1: r.y0.max(r.y1),
            },
            s,
        )
    });
    if region.is_some_and(|(r, _)| !(r.x1 > r.x0 && r.y1 > r.y0)) {
        return Err("the region is empty".into());
    }
    let mut sh = Shared::new(content.file(), cache, o.text);
    let mut b = Builder::new(&mut sh, content, cancel, v.matrix, region, None, 0);
    let drawn = content.draw(a, None, &mut b, cancel);
    if drawn.cancelled {
        return Err("cancelled".into());
    }
    let kept = b.kept;
    let root = b.finish();
    let whole = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: v.width,
        y1: v.height,
    };
    let view_box = match region {
        None => whole,
        Some((r, _)) => {
            let r = transform(&r, &v.matrix);
            match kept.filter(|_| o.fit) {
                Some(k) => intersect(&k, &r).unwrap_or(r),
                None => r,
            }
        }
    };
    let mut body = String::new();
    sh.write_node(&mut body, &root, &v.matrix, 0);
    let (w, h) = (view_box.x1 - view_box.x0, view_box.y1 - view_box.y0);
    let mut out = String::with_capacity(body.len() + sh.defs.len() + 512);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" version=\"1.1\"",
    );
    let _ = writeln!(
        out,
        " width=\"{}pt\" height=\"{}pt\" viewBox=\"{} {} {} {}\">",
        num(w),
        num(h),
        num(view_box.x0),
        num(view_box.y0),
        num(w),
        num(h)
    );
    if !sh.defs.is_empty() {
        out.push_str("<defs>\n");
        out.push_str(&sh.defs);
        out.push_str("</defs>\n");
    }
    out.push_str(&body);
    out.push_str("</svg>\n");
    Ok(Svg {
        data: out.into_bytes(),
        view_box,
        elements: sh.elements,
        images: sh.images,
        notes: sh.notes.into_iter().collect(),
    })
}

// ------------------------------------------------------------------ tree

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NodeKind {
    #[default]
    Leaf,
    Root,
    /// `q … Q`.
    Save,
    /// `BT … ET`.
    Text,
    /// `BMC` / `BDC … EMC`.
    Marked,
    /// A form XObject.
    Form,
    /// A transparency group: composited as a whole.
    Group,
}

/// An element, or a group of them, as painted.
#[derive(Default)]
struct Node {
    kind: NodeKind,
    seq: u64,
    /// Leaf: the element.
    markup: String,
    /// Leaf: text still taking runs (written at the end).
    text: Option<TextLeaf>,
    /// Group: attributes of its `<g>` (other than names and clips); a
    /// group with some is kept even with one child.
    attrs: String,
    /// Group: attributes that do not keep it (marked content tags).
    info: String,
    /// Groups: a name (layer, form).
    name: Option<String>,
    /// Optional content: a layer at the top level.
    layer: bool,
    hidden: bool,
    /// The clip when painted (leaves) or opened (groups).
    clip: Option<Arc<ClipPath>>,
    /// The soft mask it is painted with: for groups, their children's when
    /// the same for all, a transparency group's own.
    mask: Option<Arc<SoftMask>>,
    /// What it paints, unclipped, in the builder's space.
    bbox: Option<Rect>,
    children: Vec<Node>,
    /// Kept as a group even with one child.
    keep: bool,
}

/// Characters of text runs, placed one by one.
struct TextLeaf {
    /// Text element space to the builder's space.
    matrix: Matrix,
    font: Arc<Font>,
    size: f64,
    /// Font and paint attributes.
    attrs: String,
    /// The text object (`BT` count).
    object: u64,
    chars: String,
    xs: Vec<f64>,
    ys: Vec<f64>,
}

fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// The clipping paths of `clip` added since `base` (outermost first);
/// all of them when `base` is not among its parents.
fn chain(clip: &Option<Arc<ClipPath>>, base: &Option<Arc<ClipPath>>) -> Vec<Arc<ClipPath>> {
    let mut out = Vec::new();
    let mut c = clip.clone();
    while let Some(p) = c {
        if base.as_ref().is_some_and(|b| Arc::ptr_eq(b, &p)) {
            break;
        }
        c = p.parent.clone();
        out.push(p);
    }
    out.reverse();
    out
}

/// A run of children painted with the same clips and soft mask.
type Key = (Vec<Arc<ClipPath>>, Option<Arc<SoftMask>>);

/// A `<g>` around children: a clip or a soft mask.
#[derive(Clone)]
enum Wrap {
    Clip(Arc<ClipPath>),
    Mask(Arc<SoftMask>),
}

impl Wrap {
    fn same(&self, o: &Wrap) -> bool {
        match (self, o) {
            (Wrap::Clip(a), Wrap::Clip(b)) => Arc::ptr_eq(a, b),
            (Wrap::Mask(a), Wrap::Mask(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

fn same_key(a: &Key, b: &Key) -> bool {
    a.0.len() == b.0.len()
        && a.0.iter().zip(&b.0).all(|(x, y)| Arc::ptr_eq(x, y))
        && same(&a.1, &b.1)
}

// ------------------------------------------------------------- builder

/// A pattern or shading object, its space, and the color of an uncolored
/// pattern.
type PaintKey = (Option<ObjId>, [u64; 6], Option<String>);

/// State shared by the builders of a content and of its patterns and soft
/// masks: definitions and what they are made from.
struct Shared<'c> {
    file: &'c PdfFile,
    cache: &'c DocRender,
    text: TextAs,
    defs: String,
    ids: HashSet<String>,
    next: u32,
    /// Clipping path (by address, kept alive) and builder space.
    clips: HashMap<(usize, [u64; 6]), (Arc<ClipPath>, String)>,
    masks: HashMap<(ObjId, [u64; 6]), Option<String>>,
    /// Pattern or shading object, its space, and the color of an uncolored
    /// pattern.
    paints: HashMap<PaintKey, Option<String>>,
    spaces: Vec<(Arc<Object>, Arc<Space>)>,
    /// Optional content groups off by default.
    off: Option<HashSet<ObjId>>,
    notes: BTreeMap<String, u32>,
    elements: u64,
    images: u64,
}

impl<'c> Shared<'c> {
    fn new(file: &'c PdfFile, cache: &'c DocRender, text: TextAs) -> Self {
        Shared {
            file,
            cache,
            text,
            defs: String::new(),
            ids: HashSet::new(),
            next: 0,
            clips: HashMap::new(),
            masks: HashMap::new(),
            paints: HashMap::new(),
            spaces: Vec::new(),
            off: None,
            notes: BTreeMap::new(),
            elements: 0,
            images: 0,
        }
    }

    fn note(&mut self, what: impl Into<String>) {
        *self.notes.entry(what.into()).or_default() += 1;
    }

    /// A new element id.
    fn fresh(&mut self, prefix: &str) -> String {
        loop {
            self.next += 1;
            let id = format!("{prefix}{}", self.next);
            if self.ids.insert(id.clone()) {
                return id;
            }
        }
    }

    /// An id from a name (a layer's), unique.
    fn named(&mut self, name: &str) -> String {
        let mut id: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if !id.starts_with(|c: char| c.is_alphabetic() || c == '_') {
            id.insert(0, '_');
        }
        let mut k = 1;
        let mut candidate = id.clone();
        while !self.ids.insert(candidate.clone()) {
            k += 1;
            candidate = format!("{id}_{k}");
        }
        candidate
    }

    fn space(&mut self, def: &Arc<Object>) -> Arc<Space> {
        if let Some((_, s)) = self.spaces.iter().find(|(d, _)| Arc::ptr_eq(d, def)) {
            return s.clone();
        }
        let s = Arc::new(color::parse(self.file, &self.cache.profiles, def));
        if self.spaces.len() >= 64 {
            self.spaces.remove(0);
        }
        self.spaces.push((def.clone(), s.clone()));
        s
    }

    /// A color as RGB (`None`: paints nothing).
    fn rgb(&mut self, c: &Color) -> Option<[f32; 3]> {
        let space = match c.family.as_str() {
            "DeviceGray" => Arc::new(Space::Gray),
            "DeviceRGB" => Arc::new(Space::Rgb),
            "DeviceCMYK" => Arc::new(Space::Cmyk),
            _ => match &c.def {
                Some(d) => self.space(d),
                None => Arc::new(match c.n {
                    Some(3) => Space::Rgb,
                    Some(4) => Space::Cmyk,
                    _ => Space::Gray,
                }),
            },
        };
        let mut approx = Approx::default();
        let out = space.to_rgb(&c.components, &mut approx);
        if approx.tint {
            self.note("Separation / DeviceN colors are approximated");
        }
        out
    }

    /// Whether an optional content group is off by default.
    fn is_off(&mut self, id: ObjId) -> bool {
        let file = self.file;
        self.off
            .get_or_insert_with(|| {
                let d = (|| {
                    let root = resolve(file, file.trailer().get_key("Root")?).1;
                    let ocp = resolve(file, root.as_dict()?.get_key("OCProperties")?).1;
                    let d = resolve(file, ocp.as_dict()?.get_key("D")?).1;
                    let off = resolve(file, d.as_dict()?.get_key("OFF")?).1;
                    Some(
                        off.as_array()?
                            .iter()
                            .filter_map(|n| match n.obj {
                                Object::Ref(id) => Some(id),
                                _ => None,
                            })
                            .collect::<HashSet<_>>(),
                    )
                })();
                d.unwrap_or_default()
            })
            .contains(&id)
    }

    fn clip_id(&mut self, c: &Arc<ClipPath>, base: &Matrix) -> String {
        let key = (Arc::as_ptr(c) as usize, base.map(f64::to_bits));
        if let Some((_, id)) = self.clips.get(&key) {
            return id.clone();
        }
        let id = self.fresh("clip");
        let mut d = String::new();
        let _ = write!(
            d,
            "<clipPath id=\"{id}\" clipPathUnits=\"userSpaceOnUse\"><"
        );
        geometry(&mut d, &c.path, base);
        if c.even_odd {
            d.push_str(" clip-rule=\"evenodd\"");
        }
        d.push_str("/></clipPath>\n");
        self.defs.push_str(&d);
        self.clips.insert(key, (c.clone(), id.clone()));
        id
    }

    // -------------------------------------------------------- writing

    /// Write a node: a leaf's element, a group's `<g>` and children.
    fn write_node(&mut self, out: &mut String, n: &Node, base: &Matrix, depth: usize) {
        if n.kind == NodeKind::Leaf {
            match &n.text {
                Some(t) => write_text(out, t),
                None => out.push_str(&n.markup),
            }
            out.push('\n');
            self.elements += 1;
            return;
        }
        // Clips and masks relative to the group's, or (transparency
        // groups) to none: they paint from a fresh state.
        let mask_ctx = if n.kind == NodeKind::Group {
            None
        } else {
            n.mask.clone()
        };
        let keys: Vec<Key> = n
            .children
            .iter()
            .map(|c| {
                let m = c.mask.clone().filter(|_| !same(&c.mask, &mask_ctx));
                (chain(&c.clip, &n.clip), m)
            })
            .collect();
        // Everything in a group clipped (or masked) alike: on the group.
        let hoist = matches!(
            n.kind,
            NodeKind::Save | NodeKind::Text | NodeKind::Marked | NodeKind::Form
        ) && keys.windows(2).all(|w| same_key(&w[0], &w[1]))
            && keys
                .first()
                .is_some_and(|k| !k.0.is_empty() || k.1.is_some());
        let mut outer = 0;
        if n.kind != NodeKind::Root {
            let mut tag = String::from("<g");
            if let Some(name) = &n.name {
                let e = esc(name);
                if n.layer && depth == 1 {
                    let id = self.named(name);
                    let _ = write!(tag, " id=\"{}\" inkscape:groupmode=\"layer\"", esc(&id));
                }
                let _ = write!(tag, " data-name=\"{e}\" inkscape:label=\"{e}\"");
            }
            if n.hidden {
                tag.push_str(" style=\"display:none\"");
            }
            tag.push_str(&n.info);
            tag.push_str(&n.attrs);
            if hoist {
                let (clips, mask) = &keys[0];
                let clips = self.pruned(clips, n.bbox, base);
                if let Some((last, rest)) = clips.split_last() {
                    for c in rest {
                        let id = self.clip_id(c, base);
                        let _ = writeln!(out, "<g clip-path=\"url(#{id})\">");
                        outer += 1;
                    }
                    let id = self.clip_id(last, base);
                    let _ = write!(tag, " clip-path=\"url(#{id})\"");
                }
                if let Some(m) = mask {
                    if let Some(id) = self.mask_id(m, base) {
                        let _ = write!(tag, " mask=\"url(#{id})\"");
                    }
                }
            }
            out.push_str(&tag);
            out.push_str(">\n");
        }
        // Wrappers (clips, then a soft mask) stay open while the next child
        // is painted with them too: clips added one after the other nest
        // instead of starting over for each child.
        let mut open: Vec<Wrap> = Vec::new();
        for (c, key) in n.children.iter().zip(&keys) {
            let mut levels = Vec::new();
            if !hoist {
                levels.extend(
                    self.pruned(&key.0, c.bbox, base)
                        .into_iter()
                        .map(Wrap::Clip),
                );
                if let Some(m) = &key.1 {
                    levels.push(Wrap::Mask(m.clone()));
                }
            }
            let common = open
                .iter()
                .zip(&levels)
                .take_while(|(a, b)| a.same(b))
                .count();
            for _ in common..open.len() {
                out.push_str("</g>\n");
            }
            open.truncate(common);
            for l in &levels[common..] {
                match l {
                    Wrap::Clip(cp) => {
                        let id = self.clip_id(cp, base);
                        let _ = writeln!(out, "<g clip-path=\"url(#{id})\">");
                    }
                    Wrap::Mask(m) => match self.mask_id(m, base) {
                        Some(id) => {
                            let _ = writeln!(out, "<g mask=\"url(#{id})\">");
                        }
                        None => out.push_str("<g>\n"),
                    },
                }
                open.push(l.clone());
            }
            self.write_node(out, c, base, depth + 1);
        }
        for _ in 0..open.len() {
            out.push_str("</g>\n");
        }
        if n.kind != NodeKind::Root {
            out.push_str("</g>\n");
        }
        for _ in 0..outer {
            out.push_str("</g>\n");
        }
    }

    /// Clips without the rectangles that hold all of `bbox`.
    fn pruned(
        &self,
        clips: &[Arc<ClipPath>],
        bbox: Option<Rect>,
        base: &Matrix,
    ) -> Vec<Arc<ClipPath>> {
        clips
            .iter()
            .filter(|c| {
                let (Some(b), Some(r)) = (bbox, rect_of(&c.path, base)) else {
                    return true;
                };
                !(b.x0 >= r.x0 - CLIP_SLACK
                    && b.y0 >= r.y0 - CLIP_SLACK
                    && b.x1 <= r.x1 + CLIP_SLACK
                    && b.y1 <= r.y1 + CLIP_SLACK)
            })
            .cloned()
            .collect()
    }

    /// A soft mask as a `<mask>` (`None`: cannot be painted).
    fn mask_id(&mut self, sm: &Arc<SoftMask>, base: &Matrix) -> Option<String> {
        let key = (sm.id, mul(&sm.ctm, base).map(f64::to_bits));
        if let Some(id) = self.masks.get(&key) {
            return id.clone();
        }
        // Painted by a builder of its own: see `Builder::mask_def`.
        None
    }
}

/// Builds the tree of what a content paints (as the device it paints on).
struct Builder<'s, 'c> {
    sh: &'s mut Shared<'c>,
    content: &'c Content<'c>,
    cancel: &'c dyn Fn() -> bool,
    /// The content's space to SVG user space.
    base: Matrix,
    region: Option<(Rect, Select)>,
    /// Uncolored tiling pattern cells: every paint in this color.
    color: Option<String>,
    depth: u32,
    /// Open groups, the root first.
    stack: Vec<Node>,
    seq: u64,
    /// The groups of `q` (by `seq`), and of marked content.
    saves: Vec<u64>,
    marks: Vec<u64>,
    /// Open forms: their group, and the saves and marks before them.
    forms: Vec<(u64, usize, usize)>,
    /// Open text object.
    text_group: Option<u64>,
    texts: u64,
    /// `begin_group` opened a group (by `seq`).
    groups: Vec<u64>,
    /// The path of the last leaf, when a fill (a stroke may join it).
    last_fill: Option<Vec<PathEl>>,
    /// What is kept, clipped, in the builder's space.
    kept: Option<Rect>,
}

impl<'s, 'c> Builder<'s, 'c> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        sh: &'s mut Shared<'c>,
        content: &'c Content<'c>,
        cancel: &'c dyn Fn() -> bool,
        base: Matrix,
        region: Option<(Rect, Select)>,
        color: Option<String>,
        depth: u32,
    ) -> Self {
        Builder {
            sh,
            content,
            cancel,
            base,
            region,
            color,
            depth,
            stack: vec![Node {
                kind: NodeKind::Root,
                keep: true,
                ..Node::default()
            }],
            seq: 0,
            saves: Vec::new(),
            marks: Vec::new(),
            forms: Vec::new(),
            text_group: None,
            texts: 0,
            groups: Vec::new(),
            last_fill: None,
            kept: None,
        }
    }

    fn note(&mut self, what: impl Into<String>) {
        self.sh.note(what);
    }

    fn open(&mut self, mut n: Node, g: &GState) -> u64 {
        self.seq += 1;
        n.seq = self.seq;
        n.clip = g.clip_path.clone();
        if n.kind != NodeKind::Group {
            n.mask = None;
        }
        self.stack.push(n);
        self.last_fill = None;
        self.seq
    }

    /// Close the group `seq` and those opened inside it.
    fn close(&mut self, seq: u64) {
        let Some(k) = self.stack.iter().rposition(|n| n.seq == seq) else {
            return;
        };
        if k == 0 {
            return;
        }
        while self.stack.len() > k {
            let n = self.stack.pop().expect("above the root");
            self.attach(n);
        }
        self.last_fill = None;
    }

    /// Put a closed group (or leaf) into the open one.
    fn attach(&mut self, mut n: Node) {
        if n.kind != NodeKind::Leaf {
            if n.children.is_empty() {
                return;
            }
            n.bbox = n
                .children
                .iter()
                .filter_map(|c| c.bbox)
                .fold(None, |a, b| Some(union(a, b)));
            if n.kind != NodeKind::Group {
                let first = n.children[0].mask.clone();
                n.mask = first.filter(|_| {
                    n.children
                        .iter()
                        .all(|c| same(&c.mask, &n.children[0].mask))
                });
            }
            // A transparency group's soft mask is its own: not its child's.
            let own_mask = n.kind == NodeKind::Group && n.mask.is_some();
            if !n.keep && n.children.len() == 1 && n.attrs.is_empty() && !own_mask {
                let mut c = n.children.pop().expect("one child");
                if c.kind != NodeKind::Leaf && c.name.is_none() {
                    c.name = n.name.take();
                }
                return self.attach(c);
            }
        }
        self.stack
            .last_mut()
            .expect("the root stays")
            .children
            .push(n);
    }

    fn finish(mut self) -> Node {
        while self.stack.len() > 1 {
            let n = self.stack.pop().expect("above the root");
            self.attach(n);
        }
        self.stack.pop().expect("the root")
    }

    /// Whether something painting `painted` (the content's space) is kept;
    /// its box in the builder's space.
    fn admit(&mut self, g: &GState, painted: Rect) -> Option<Rect> {
        let visible = match g.clip {
            Some(c) => intersect(&painted, &c)?,
            None => painted,
        };
        if let Some((r, s)) = &self.region {
            let ok = match s {
                Select::Inside => {
                    visible.x0 >= r.x0 - EPSILON
                        && visible.y0 >= r.y0 - EPSILON
                        && visible.x1 <= r.x1 + EPSILON
                        && visible.y1 <= r.y1 + EPSILON
                }
                Select::Overlaps => intersect(&visible, r).is_some(),
            };
            if !ok {
                return None;
            }
        }
        self.kept = Some(union(self.kept, transform(&visible, &self.base)));
        Some(transform(&painted, &self.base))
    }

    fn leaf(&mut self, markup: String, g: &GState, bbox: Rect) {
        self.push(Node {
            markup,
            clip: g.clip_path.clone(),
            mask: g.smask.clone(),
            bbox: Some(bbox),
            ..Node::default()
        });
    }

    fn push(&mut self, n: Node) {
        self.last_fill = None;
        self.stack
            .last_mut()
            .expect("the root stays")
            .children
            .push(n);
    }

    // ------------------------------------------------------ paints

    /// What a color paints with, for an element whose user space `elem`
    /// maps to the builder's space and which covers `area` there: a color,
    /// or a gradient or pattern; `None` paints nothing.
    fn paint(&mut self, c: &Color, g: &GState, elem: &Matrix, area: Rect) -> Option<String> {
        if let Some(o) = &self.color {
            return Some(o.clone());
        }
        if c.family == "Pattern" {
            return self.pattern(c, g, elem, area);
        }
        self.sh.rgb(c).map(hex)
    }

    fn pattern(&mut self, c: &Color, g: &GState, elem: &Matrix, area: Rect) -> Option<String> {
        let Some(def) = c.pattern_def.clone() else {
            self.note("a pattern color names no pattern: nothing is painted");
            return None;
        };
        let (pid, o) = &*def;
        let d = match o {
            Object::Dict(d) => d,
            Object::Stream(s) => &s.dict,
            _ => return None,
        };
        let file = self.sh.file;
        let matrix = d
            .get_key("Matrix")
            .and_then(|m| crate::function::nums(file, Some(m)))
            .and_then(|m| <[f64; 6]>::try_from(m).ok())
            .unwrap_or(IDENTITY);
        let to_base = mul(&mul(&matrix, &g.pattern_ctm), &self.base);
        let m = mul(&to_base, &invert(elem)?);
        match d.get_i64("PatternType") {
            Some(2) => {
                let so = d.get_key("Shading")?;
                let (sid, _) = resolve(file, so);
                let key = sid.map(|i| (i, false)).or(pid.map(|i| (i, true)));
                let sh = self.shading_of(key, sid, so);
                let sh = match &*sh {
                    Ok(s) => s,
                    Err(e) => {
                        self.note(format!("shading not drawn: {e}"));
                        return None;
                    }
                };
                if let Some(gr) = sh.gradient() {
                    return self.gradient_def(sid.or(*pid), &gr, &m).map(url);
                }
                // As an image over what is painted.
                let png = self.shading_png(sh, &to_base, area, true)?;
                let id = self.sh.fresh("pattern");
                let _ = write!(
                    self.sh.defs,
                    "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"",
                    x = num(area.x0),
                    y = num(area.y0),
                    w = num(area.x1 - area.x0),
                    h = num(area.y1 - area.y0),
                );
                if *elem != IDENTITY {
                    let _ = write!(
                        self.sh.defs,
                        " patternTransform=\"{}\"",
                        matrix_text(&invert(elem)?)
                    );
                }
                let _ = writeln!(
                    self.sh.defs,
                    "><image x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\" xlink:href=\"data:image/png;base64,{png}\"/></pattern>",
                    x = num(area.x0),
                    y = num(area.y0),
                    w = num(area.x1 - area.x0),
                    h = num(area.y1 - area.y0),
                );
                self.note("a shading pattern is drawn as an image");
                Some(url(id))
            }
            Some(1) => match (pid, o) {
                (Some(id), Object::Stream(s)) => self.tiling(*id, s, c, &m).map(url),
                _ => None,
            },
            _ => {
                self.note("a pattern of unknown /PatternType: nothing is painted");
                None
            }
        }
    }

    fn shading_of(
        &self,
        key: Option<(ObjId, bool)>,
        id: Option<ObjId>,
        o: &Object,
    ) -> Arc<Result<Shading, String>> {
        let (file, cache) = (self.sh.file, self.sh.cache);
        let parse = || Arc::new(Shading::parse(file, &cache.profiles, id, o));
        match key {
            Some(k) => cache.shading(k, parse),
            None => parse(),
        }
    }

    /// A gradient (`m`: the shading's space to the user space it is used
    /// in).
    fn gradient_def(&mut self, id: Option<ObjId>, gr: &Gradient, m: &Matrix) -> Option<String> {
        let key = (id, m.map(f64::to_bits), None);
        if let Some(d) = id.and(self.sh.paints.get(&key)) {
            return d.clone();
        }
        if gr.approximate {
            self.note("Separation / DeviceN colors are approximated");
        }
        let gid = self.sh.fresh("gradient");
        let mut d = String::new();
        let mut stops = gr.stops.clone();
        let mut extend = gr.extend;
        match gr.kind {
            GradientKind::Axial(c) => {
                let _ = write!(
                    d,
                    "<linearGradient id=\"{gid}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                    num(c[0]),
                    num(c[1]),
                    num(c[2]),
                    num(c[3])
                );
            }
            GradientKind::Radial(c) => {
                // The focal circle is the start circle, inside the end one;
                // the other way round, the stops run backwards.
                let (f, e) = if c[2] <= c[5] {
                    ([c[0], c[1], c[2]], [c[3], c[4], c[5]])
                } else {
                    stops = stops.iter().rev().map(|&(o, c)| (1.0 - o, c)).collect();
                    extend = (extend.1, extend.0);
                    ([c[3], c[4], c[5]], [c[0], c[1], c[2]])
                };
                let _ = write!(
                    d,
                    "<radialGradient id=\"{gid}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\" fx=\"{}\" fy=\"{}\"",
                    num(e[0]),
                    num(e[1]),
                    num(e[2]),
                    num(f[0]),
                    num(f[1])
                );
                if f[2] > 0.0 {
                    let _ = write!(d, " fr=\"{}\"", num(f[2]));
                }
            }
        }
        if *m != IDENTITY {
            let _ = write!(d, " gradientTransform=\"{}\"", matrix_text(m));
        }
        d.push('>');
        // Not extended: nothing beyond the ends (the padding is clear).
        let stop = |d: &mut String, o: f64, c: [u8; 4], clear: bool| {
            let _ = write!(
                d,
                "<stop offset=\"{}\" stop-color=\"{}\"",
                num(o.clamp(0.0, 1.0)),
                hex([c[0], c[1], c[2]].map(|v| v as f32 / 255.0))
            );
            if clear || c[3] < 255 {
                let _ = write!(
                    d,
                    " stop-opacity=\"{}\"",
                    if clear { 0.0 } else { c[3] as f64 / 255.0 }
                );
            }
            d.push_str("/>");
        };
        if let (false, Some(&(o, c))) = (extend.0, stops.first()) {
            stop(&mut d, o, c, true);
        }
        for &(o, c) in &stops {
            stop(&mut d, o, c, false);
        }
        if let (false, Some(&(o, c))) = (extend.1, stops.last()) {
            stop(&mut d, o, c, true);
        }
        d.push_str(match gr.kind {
            GradientKind::Axial(_) => "</linearGradient>\n",
            GradientKind::Radial(_) => "</radialGradient>\n",
        });
        self.sh.defs.push_str(&d);
        if id.is_some() {
            self.sh.paints.insert(key, Some(gid.clone()));
        }
        Some(gid)
    }

    /// A shading painted over `area` of the builder's space (`m`: its space
    /// to the builder's), as base64 PNG.
    fn shading_png(
        &mut self,
        sh: &Shading,
        m: &Matrix,
        area: Rect,
        background: bool,
    ) -> Option<String> {
        let (w, h) = (area.x1 - area.x0, area.y1 - area.y0);
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let k = SHADING_SCALE.min((SHADING_PIXELS / (w * h)).sqrt());
        let (pw, ph) = (
            (w * k).ceil().max(1.0) as i32,
            (h * k).ceil().max(1.0) as i32,
        );
        let to_px = mul(m, &[k, 0.0, 0.0, k, -area.x0 * k, -area.y0 * k]);
        if sh.approximate {
            self.note("Separation / DeviceN colors are approximated");
        }
        let pix = sh.paint(&to_px, (0, 0, pw, ph), background)?;
        Some(base64(&png_of(&pix)))
    }

    /// A tiling pattern as a `<pattern>` (`m`: pattern space to the user
    /// space it is used in).
    fn tiling(&mut self, id: ObjId, s: &Stream, c: &Color, m: &Matrix) -> Option<String> {
        let file = self.sh.file;
        let d = &s.dict;
        let Some(bbox) = d.get_key("BBox").and_then(|b| Rect::read(file, b)) else {
            self.note("a tiling pattern without a valid /BBox: nothing is painted");
            return None;
        };
        let num_of = |k: &str| d.get_key(k).and_then(|v| resolve(file, v).1.as_f64());
        let (xs, ys) = (
            num_of("XStep").unwrap_or(0.0).abs(),
            num_of("YStep").unwrap_or(0.0).abs(),
        );
        if !(xs > 0.0 && ys > 0.0 && xs.is_finite() && ys.is_finite()) {
            self.note("a tiling pattern with a zero /XStep or /YStep: nothing is painted");
            return None;
        }
        // An uncolored pattern takes its color from the base space.
        let color = if d.get_i64("PaintType") == Some(2) {
            let space = c.def.as_ref().map(|d| self.sh.space(d));
            let rgb = match space.as_deref() {
                Some(Space::Pattern(Some(base))) => {
                    base.to_rgb(&c.components, &mut Approx::default())
                }
                _ => None,
            };
            let Some(rgb) = rgb else {
                self.note(
                    "an uncolored tiling pattern without a base color space: nothing is painted",
                );
                return None;
            };
            Some(hex(rgb))
        } else {
            None
        };
        let key = (Some(id), m.map(f64::to_bits), color.clone());
        if let Some(p) = self.sh.paints.get(&key) {
            return p.clone();
        }
        if self.depth >= MAX_DEPTH {
            self.note("patterns nested too deeply: nothing is painted");
            return None;
        }
        let cell = {
            let mut sub = Builder::new(
                &mut *self.sh,
                self.content,
                self.cancel,
                IDENTITY,
                None,
                color,
                self.depth + 1,
            );
            let mut g = GState::new(IDENTITY, Some(bbox));
            g.clip_path = Some(Arc::new(ClipPath {
                path: rect_els(&bbox),
                even_odd: false,
                parent: None,
            }));
            self.content
                .draw_stream(id, s, g, Kind::Pattern, &mut sub, self.cancel);
            let root = sub.finish();
            let mut out = String::new();
            self.sh.write_node(&mut out, &root, &IDENTITY, 1);
            out
        };
        let pid = self.sh.fresh("pattern");
        let cid = self.sh.fresh("cell");
        let mut p = String::new();
        let _ = write!(
            p,
            "<pattern id=\"{pid}\" patternUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            num(bbox.x0),
            num(bbox.y0),
            num(xs),
            num(ys)
        );
        if *m != IDENTITY {
            let _ = write!(p, " patternTransform=\"{}\"", matrix_text(m));
        }
        let _ = write!(p, ">\n<g id=\"{cid}\">\n{cell}</g>\n");
        // Content beyond the cell (a box larger than the step) wraps
        // around.
        let nx = (((bbox.x1 - bbox.x0) / xs).ceil() as i32).clamp(1, 3);
        let ny = (((bbox.y1 - bbox.y0) / ys).ceil() as i32).clamp(1, 3);
        for i in 0..nx {
            for j in 0..ny {
                if i > 0 || j > 0 {
                    let _ = writeln!(
                        p,
                        "<use xlink:href=\"#{cid}\" transform=\"translate({} {})\"/>",
                        num(-i as f64 * xs),
                        num(-j as f64 * ys)
                    );
                }
            }
        }
        p.push_str("</pattern>\n");
        self.sh.defs.push_str(&p);
        self.sh.paints.insert(key, Some(pid.clone()));
        Some(pid)
    }

    /// Make sure a soft mask has its `<mask>` (see [`Shared::mask_id`]).
    fn mask_def(&mut self, sm: &Arc<SoftMask>) {
        let key = (sm.id, mul(&sm.ctm, &self.base).map(f64::to_bits));
        if self.sh.masks.contains_key(&key) {
            return;
        }
        let id = self.render_mask(sm);
        if id.is_none() {
            self.note("a soft mask could not be written and is left out");
        }
        self.sh.masks.insert(key, id);
    }

    fn render_mask(&mut self, sm: &SoftMask) -> Option<String> {
        let s = sm.group.as_stream()?;
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let file = self.sh.file;
        let d = &s.dict;
        let matrix = d
            .get_key("Matrix")
            .and_then(|m| crate::function::nums(file, Some(m)))
            .and_then(|m| <[f64; 6]>::try_from(m).ok())
            .unwrap_or(IDENTITY);
        let bbox = d.get_key("BBox").and_then(|b| Rect::read(file, b))?;
        let base = mul(&mul(&matrix, &sm.ctm), &self.base);
        let area = transform(&bbox, &base);
        let backdrop = if sm.luminosity {
            let space = d
                .get_key("Group")
                .and_then(|gr| resolve(file, gr).1.as_dict().cloned())
                .and_then(|gr| {
                    gr.get_key("CS")
                        .map(|cs| color::parse(file, &self.sh.cache.profiles, cs))
                });
            let rgb = match (&sm.backdrop, space) {
                (Some(bc), Some(space)) => space.to_rgb(bc, &mut Approx::default()),
                _ => None,
            };
            Some(hex(rgb.unwrap_or([0.0; 3])))
        } else {
            None
        };
        // The transfer function: a filter over the mask's values (gray
        // from the luminosity first).
        let transfer = sm.transfer.as_ref().and_then(|t| {
            let f = crate::function::Function::parse(file, t);
            if f.is_none() {
                self.note("a soft mask transfer function cannot be read and is left out");
            }
            let f = f?;
            let table: Vec<String> = (0..=TRANSFER_SAMPLES)
                .map(|k| {
                    let v = k as f64 / TRANSFER_SAMPLES as f64;
                    let o = f.eval(&[v]).and_then(|o| o.first().copied()).unwrap_or(v);
                    num(o.clamp(0.0, 1.0))
                })
                .collect();
            Some(table.join(" "))
        });
        let inner = {
            let mut sub = Builder::new(
                &mut *self.sh,
                self.content,
                self.cancel,
                base,
                None,
                None,
                self.depth + 1,
            );
            let mut g = GState::new(IDENTITY, Some(bbox));
            g.clip_path = Some(Arc::new(ClipPath {
                path: rect_els(&bbox),
                even_odd: false,
                parent: None,
            }));
            self.content
                .draw_stream(sm.id, s, g, Kind::Form, &mut sub, self.cancel);
            let root = sub.finish();
            let mut out = String::new();
            self.sh.write_node(&mut out, &root, &base, 1);
            out
        };
        let id = self.sh.fresh("mask");
        let (x, y, w, h) = (
            num(area.x0),
            num(area.y0),
            num(area.x1 - area.x0),
            num(area.y1 - area.y0),
        );
        let _ = write!(
            self.sh.defs,
            "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\""
        );
        if backdrop.is_none() {
            self.sh.defs.push_str(" style=\"mask-type:alpha\"");
        }
        self.sh.defs.push_str(">\n");
        if let Some(table) = &transfer {
            let fid = self.sh.fresh("transfer");
            let _ = write!(
                self.sh.defs,
                "<filter id=\"{fid}\" filterUnits=\"userSpaceOnUse\" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" color-interpolation-filters=\"sRGB\">"
            );
            if backdrop.is_some() {
                let l = "0.3 0.59 0.11 0 0";
                let _ = write!(
                    self.sh.defs,
                    "<feColorMatrix type=\"matrix\" values=\"{l} {l} {l} 0 0 0 1 0\"/><feComponentTransfer>\
                     <feFuncR type=\"table\" tableValues=\"{table}\"/><feFuncG type=\"table\" tableValues=\"{table}\"/>\
                     <feFuncB type=\"table\" tableValues=\"{table}\"/></feComponentTransfer>"
                );
            } else {
                let _ = write!(
                    self.sh.defs,
                    "<feComponentTransfer><feFuncA type=\"table\" tableValues=\"{table}\"/></feComponentTransfer>"
                );
            }
            let _ = writeln!(self.sh.defs, "</filter>\n<g filter=\"url(#{fid})\">");
        }
        if let Some(bc) = &backdrop {
            let _ = writeln!(
                self.sh.defs,
                "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"{bc}\"/>"
            );
        }
        self.sh.defs.push_str(&inner);
        if transfer.is_some() {
            self.sh.defs.push_str("</g>\n");
        }
        self.sh.defs.push_str("</mask>\n");
        Some(id)
    }

    /// Paint attributes common to elements: opacity and blend mode.
    fn blend(&mut self, out: &mut String, g: &GState) {
        self.blend_with(out, g, None);
    }

    /// Same, with more of a `style`.
    fn blend_with(&mut self, out: &mut String, g: &GState, style: Option<&str>) {
        let mut css: Vec<String> = style.into_iter().map(String::from).collect();
        if g.blend != "Normal" && g.blend != "Compatible" {
            match blend_mode(&g.blend) {
                Some(b) => css.push(format!("mix-blend-mode:{b}")),
                None => self.note(format!(
                    "unknown blend mode /{} is written as normal",
                    g.blend
                )),
            }
        }
        if !css.is_empty() {
            let _ = write!(out, " style=\"{}\"", css.join(";"));
        }
        if let Some(sm) = &g.smask {
            self.mask_def(sm);
        }
    }

    /// Stroke attributes (`k`: how much user space lengths scale).
    fn stroke_attrs(&self, out: &mut String, paint: &str, g: &GState, k: f64, alpha: f64) {
        let mut w = g.line_width.abs() * k;
        if w == 0.0 {
            // A hairline: the thinnest line, about a quarter point.
            let s = scale_of(&mul(&g.ctm, &self.base)).max(1e-9) / k.max(1e-9);
            w = HAIRLINE / s;
        }
        let _ = write!(out, " stroke=\"{paint}\" stroke-width=\"{}\"", num(w));
        match g.line_cap {
            1 => out.push_str(" stroke-linecap=\"round\""),
            2 => out.push_str(" stroke-linecap=\"square\""),
            _ => {}
        }
        match g.line_join {
            1 => out.push_str(" stroke-linejoin=\"round\""),
            2 => out.push_str(" stroke-linejoin=\"bevel\""),
            _ => {
                if g.miter_limit != 4.0 {
                    let _ = write!(
                        out,
                        " stroke-miterlimit=\"{}\"",
                        num(g.miter_limit.max(1.0))
                    );
                }
            }
        }
        if !g.dash.is_empty()
            && g.dash.iter().all(|&d| d >= 0.0)
            && g.dash.iter().sum::<f64>() > 0.0
        {
            out.push_str(" stroke-dasharray=\"");
            for (i, d) in g.dash.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(&num(d * k));
            }
            out.push('"');
            if g.dash_phase != 0.0 {
                let _ = write!(out, " stroke-dashoffset=\"{}\"", num(g.dash_phase * k));
            }
        }
        if alpha < 1.0 {
            let _ = write!(out, " stroke-opacity=\"{}\"", num(alpha.max(0.0)));
        }
    }

    // --------------------------------------------------------- text

    fn text_leaf(
        &mut self,
        run: &GlyphRun<'_>,
        g: &GState,
        bbox: Rect,
        glyphs: &[(&Glyph, String)],
    ) {
        let font = &run.font.info;
        let sign = if run.size < 0.0 { -1.0 } else { 1.0 };
        let th = run.scale;
        let matrix = mul(
            &mul(&[sign * th, 0.0, 0.0, -sign, 0.0, 0.0], &run.matrix),
            &self.base,
        );
        let r = g.text.render;
        let mut attrs = font_attrs(font, run.size.abs());
        let paint_fill = if matches!(r, 0 | 2 | 4 | 6) {
            self.paint(&g.fill, g, &matrix, bbox)
        } else {
            None
        };
        match &paint_fill {
            Some(p) => {
                if p != "#000000" {
                    let _ = write!(attrs, " fill=\"{p}\"");
                }
                if g.fill_alpha < 1.0 {
                    let _ = write!(attrs, " fill-opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
                }
            }
            None => attrs.push_str(" fill=\"none\""),
        }
        if matches!(r, 1 | 2 | 5 | 6) {
            if let Some(p) = self.paint(&g.stroke, g, &matrix, bbox) {
                let k = scale_of(&mul(&g.ctm, &self.base)) / scale_of(&matrix).max(1e-12);
                self.stroke_attrs(&mut attrs, &p, g, k, g.stroke_alpha);
            }
        }
        self.blend(&mut attrs, g);
        let (mut chars, mut xs, mut ys) = (String::new(), Vec::new(), Vec::new());
        for (gl, t) in glyphs {
            let (x, y) = (sign * gl.x / th, -sign * gl.y);
            let n = t.chars().count().max(1) as f64;
            let adv = (gl.bbox.x1 - gl.bbox.x0) / th.abs();
            for (k, ch) in t.chars().enumerate() {
                chars.push(ch);
                xs.push(x + sign * adv * k as f64 / n);
                ys.push(y);
            }
        }
        let object = self.texts;
        // A run of the same text object on the same line, with the same
        // font and paint, continues the last one.
        if let Some(Node {
            text: Some(t),
            clip,
            mask,
            bbox: b,
            ..
        }) = self.stack.last_mut().and_then(|n| n.children.last_mut())
        {
            let rel = mul(&matrix, &invert(&t.matrix).unwrap_or(IDENTITY));
            let lin = (rel[0] - 1.0).abs() + rel[1].abs() + rel[2].abs() + (rel[3] - 1.0).abs();
            if t.object == object
                && Arc::ptr_eq(&t.font, font)
                && t.size == run.size.abs()
                && t.attrs == attrs
                && same(clip, &g.clip_path)
                && same(mask, &g.smask)
                && lin < 1e-6
                && rel[5].abs() < 0.01 * t.size.max(1e-3)
            {
                t.chars.push_str(&chars);
                t.xs.extend(xs.iter().map(|x| x + rel[4]));
                t.ys.extend(ys.iter().map(|y| y + rel[5]));
                *b = Some(union(*b, bbox));
                self.last_fill = None;
                return;
            }
        }
        self.push(Node {
            text: Some(TextLeaf {
                matrix,
                font: font.clone(),
                size: run.size.abs(),
                attrs,
                object,
                chars,
                xs,
                ys,
            }),
            clip: g.clip_path.clone(),
            mask: g.smask.clone(),
            bbox: Some(bbox),
            ..Node::default()
        });
    }

    /// Glyphs as outlines, in one group named by their text.
    fn outline_leaf(&mut self, run: &GlyphRun<'_>, g: &GState, bbox: Rect, glyphs: &[&Glyph]) {
        let file = self.sh.file;
        let font = &run.font.info;
        let mut paths = Vec::new();
        let mut label = String::new();
        for gl in glyphs {
            let mut els = Vec::new();
            run.glyph_path(file, gl, &mut els);
            if !els.is_empty() {
                let mut d = String::new();
                path_data(&mut d, &els, &self.base);
                paths.push(d);
            }
            if let Some(t) = font.text(gl.code) {
                label.push_str(&t);
            }
        }
        if paths.is_empty() {
            return;
        }
        let r = g.text.render;
        let mut attrs = String::new();
        if !label.trim().is_empty() {
            let _ = write!(attrs, " aria-label=\"{}\"", esc(&label));
        }
        match matches!(r, 0 | 2 | 4 | 6)
            .then(|| self.paint(&g.fill, g, &IDENTITY, bbox))
            .flatten()
        {
            Some(p) => {
                let _ = write!(attrs, " fill=\"{p}\"");
                if g.fill_alpha < 1.0 {
                    let _ = write!(attrs, " fill-opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
                }
            }
            None => attrs.push_str(" fill=\"none\""),
        }
        if matches!(r, 1 | 2 | 5 | 6) {
            if let Some(p) = self.paint(&g.stroke, g, &IDENTITY, bbox) {
                let k = scale_of(&mul(&g.ctm, &self.base));
                self.stroke_attrs(&mut attrs, &p, g, k, g.stroke_alpha);
            }
        }
        self.blend(&mut attrs, g);
        let mut m = String::new();
        if paths.len() == 1 {
            let _ = write!(m, "<path d=\"{}\"{attrs}/>", paths[0]);
        } else {
            let _ = write!(m, "<g{attrs}>");
            for d in &paths {
                let _ = write!(m, "<path d=\"{d}\"/>");
            }
            m.push_str("</g>");
        }
        self.leaf(m, g, bbox);
    }

    // ------------------------------------------------------- images

    fn decoded(
        &self,
        img: ImageData<'_>,
        res: &Resources,
    ) -> Arc<Result<Decoded, image::ImageError>> {
        let (file, cache) = (self.sh.file, self.sh.cache);
        match img {
            ImageData::XObject {
                id: Some(id),
                stream,
            } => cache.image(file, id, stream, 1),
            ImageData::XObject { id: None, stream } => Arc::new(image::decode_xobject(
                file,
                &cache.profiles,
                None,
                stream,
                1,
            )),
            ImageData::Inline { dict, data } => {
                Arc::new(image::decode_inline(file, &cache.profiles, dict, data, res))
            }
        }
    }

    /// The JPEG file an image XObject stores, when it shows as it paints.
    fn jpeg(&self, img: ImageData<'_>) -> Option<Vec<u8>> {
        let ImageData::XObject {
            id: Some(id),
            stream,
        } = img
        else {
            return None;
        };
        let d = &stream.dict;
        if d.get_key("SMask").is_some() || d.get_key("Mask").is_some() {
            return None;
        }
        let file = self.sh.file;
        if self.sh.cache.stored_file(file, stream) != Some(StoredFile::Jpeg) {
            return None;
        }
        let raw = file.decode_stream(id, stream, Some(MAX_JPEG));
        (raw.stopped_at.is_some() && raw.errors.is_empty() && raw.data.starts_with(&[0xff, 0xd8]))
            .then_some(raw.data)
    }
}

impl Device for Builder<'_, '_> {
    fn fill(&mut self, path: &[PathEl], even_odd: bool, g: &GState) {
        self.last_fill = None;
        let Some(pb) = path_box(path) else { return };
        let Some(bbox) = self.admit(g, pb) else {
            return;
        };
        let Some(paint) = self.paint(&g.fill, g, &IDENTITY, bbox) else {
            return;
        };
        let mut m = String::from("<");
        geometry(&mut m, path, &self.base);
        let _ = write!(m, " fill=\"{paint}\"");
        if even_odd {
            m.push_str(" fill-rule=\"evenodd\"");
        }
        if g.fill_alpha < 1.0 {
            let _ = write!(m, " fill-opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
        }
        self.blend(&mut m, g);
        m.push_str("/>");
        self.leaf(m, g, bbox);
        self.last_fill = Some(path.to_vec());
    }

    fn stroke(&mut self, path: &[PathEl], g: &GState) {
        let Some(pb) = path_box(path) else { return };
        let pb = grow(&pb, g.line_width.abs() * scale_of(&g.ctm) / 2.0);
        let Some(bbox) = self.admit(g, pb) else {
            return;
        };
        let m = mul(&g.ctm, &self.base);
        let similar = similarity(&m);
        let elem = if similar { IDENTITY } else { m };
        let k = if similar { scale_of(&m) } else { 1.0 };
        let Some(paint) = self.paint(&g.stroke, g, &elem, bbox) else {
            return;
        };
        let mut attrs = String::new();
        self.stroke_attrs(&mut attrs, &paint, g, k, g.stroke_alpha);
        // `B`: one element filled and stroked.
        if similar && self.last_fill.as_deref() == Some(path) {
            let top = self.stack.last_mut().expect("the root stays");
            if let Some(last) = top.children.last_mut() {
                if same(&last.clip, &g.clip_path) && last.markup.ends_with("/>") {
                    last.markup.truncate(last.markup.len() - 2);
                    last.markup.push_str(&attrs);
                    last.markup.push_str("/>");
                    last.bbox = Some(union(last.bbox, bbox));
                    self.last_fill = None;
                    return;
                }
            }
        }
        let mut s = String::from("<");
        if similar {
            geometry(&mut s, path, &self.base);
        } else {
            let Some(inv) = invert(&g.ctm) else { return };
            let user: Vec<PathEl> = path.iter().map(|e| map_el(e, &inv)).collect();
            s.push_str("path d=\"");
            path_data(&mut s, &user, &IDENTITY);
            let _ = write!(s, "\" transform=\"{}\"", matrix_text(&m));
        }
        s.push_str(" fill=\"none\"");
        s.push_str(&attrs);
        self.blend(&mut s, g);
        s.push_str("/>");
        self.leaf(s, g, bbox);
    }

    fn text(&mut self, run: &GlyphRun<'_>, g: &GState) {
        let file = self.sh.file;
        let font = &run.font.info;
        if run.scale == 0.0 || run.size == 0.0 {
            return;
        }
        let Some(mut pb) = run
            .glyphs
            .iter()
            .map(|gl| transform(&gl.bbox, &run.matrix))
            .reduce(|a, b| union(Some(a), b))
        else {
            return;
        };
        if matches!(g.text.render, 1 | 2 | 5 | 6) {
            pb = grow(&pb, g.line_width.abs() * scale_of(&g.ctm) / 2.0);
        }
        let Some(bbox) = self.admit(g, pb) else {
            return;
        };
        let outlines = font.has_outlines(file);
        let as_text = self.sh.text == TextAs::Text || !outlines;
        if !outlines && self.sh.text == TextAs::Paths {
            let name = font
                .base_font
                .as_deref()
                .unwrap_or(&run.font.name)
                .to_string();
            self.note(format!(
                "font {name} has no glyphs: its text is written as text"
            ));
        }
        let mut texts = Vec::new();
        let mut rest = Vec::new();
        for gl in &run.glyphs {
            let t = as_text
                .then(|| font.text(gl.code))
                .flatten()
                .filter(|t| !t.is_empty() && t.chars().all(xml_char));
            match t {
                Some(t) => texts.push((gl, t)),
                None => rest.push(gl),
            }
        }
        if !texts.is_empty() {
            self.text_leaf(run, g, bbox, &texts);
        }
        if !rest.is_empty() {
            if outlines {
                if as_text {
                    self.note("glyphs without Unicode text are written as outlines");
                }
                self.outline_leaf(run, g, bbox, &rest);
            } else {
                self.note("glyphs without Unicode text or outlines are left out");
            }
        }
    }

    fn image(&mut self, img: ImageData<'_>, res: &Resources, g: &GState) {
        let Some(bbox) = self.admit(g, transform(&unit_square(), &g.ctm)) else {
            return;
        };
        let decoded = self.decoded(img, res);
        let d = match &*decoded {
            Ok(d) => d,
            Err(e) => {
                self.note(format!("image left out: {e}"));
                return;
            }
        };
        if d.approximate {
            self.note("Separation / DeviceN colors are approximated");
        }
        let (w, h) = (d.pixmap.width(), d.pixmap.height());
        let m = mul(
            &[1.0 / w as f64, 0.0, 0.0, -1.0 / h as f64, 0.0, 1.0],
            &mul(&g.ctm, &self.base),
        );
        let pattern = d.stencil && g.fill.family == "Pattern" && self.color.is_none();
        let href = if d.stencil {
            // In the fill color (white under a pattern: the mask's
            // coverage).
            let rgb = if pattern {
                Some([1.0; 3])
            } else {
                match &self.color {
                    Some(c) => parse_hex(c),
                    None => self.sh.rgb(&g.fill),
                }
            };
            let Some(rgb) = rgb else { return };
            let mut p = d.pixmap.clone();
            let q = |v: f32, a: u8| ((v * a as f32) + 0.5) as u8;
            for px in p.data_mut().chunks_exact_mut(4) {
                let a = px[3];
                px[0] = q(rgb[0], a);
                px[1] = q(rgb[1], a);
                px[2] = q(rgb[2], a);
            }
            format!("data:image/png;base64,{}", base64(&png_of(&p)))
        } else {
            match self.jpeg(img) {
                Some(j) => format!("data:image/jpeg;base64,{}", base64(&j)),
                None => format!("data:image/png;base64,{}", base64(&png_of(&d.pixmap))),
            }
        };
        let mut s = format!(
            "<image width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\" transform=\"{}\"",
            matrix_text(&m)
        );
        // Pixels larger than a point and no `/Interpolate`: as squares.
        let (dw, dh) = (m[0].hypot(m[1]) * w as f64, m[2].hypot(m[3]) * h as f64);
        let pixelated = !d.interpolate && (dw / w as f64 > 1.5 || dh / h as f64 > 1.5);
        if pixelated {
            s.push_str(" image-rendering=\"optimizeSpeed\"");
        }
        if g.fill_alpha < 1.0 && !pattern {
            let _ = write!(s, " opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
        }
        let style = pixelated.then_some("image-rendering:pixelated");
        if pattern {
            if let Some(st) = style {
                let _ = write!(s, " style=\"{st}\"");
            }
        } else {
            self.blend_with(&mut s, g, style);
        }
        let _ = write!(s, " xlink:href=\"{href}\"/>");
        self.sh.images += 1;
        if pattern {
            // A stencil mask filled with a pattern: the pattern through
            // the mask.
            let Some(paint) = self.paint(&g.fill, g, &IDENTITY, bbox) else {
                return;
            };
            let id = self.sh.fresh("mask");
            let _ = writeln!(
                self.sh.defs,
                "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{s}</mask>",
                num(bbox.x0),
                num(bbox.y0),
                num(bbox.x1 - bbox.x0),
                num(bbox.y1 - bbox.y0)
            );
            let mut r = String::new();
            let _ = write!(
                r,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{paint}\" mask=\"url(#{id})\"",
                num(bbox.x0),
                num(bbox.y0),
                num(bbox.x1 - bbox.x0),
                num(bbox.y1 - bbox.y0)
            );
            if g.fill_alpha < 1.0 {
                let _ = write!(r, " fill-opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
            }
            self.blend(&mut r, g);
            r.push_str("/>");
            return self.leaf(r, g, bbox);
        }
        self.leaf(s, g, bbox);
    }

    fn shading(&mut self, id: Option<ObjId>, shading: &Object, g: &GState) {
        let Some(area) = g.clip else {
            self.note("a shading without a clip is left out");
            return;
        };
        let Some(bbox) = self.admit(g, area) else {
            return;
        };
        let sh = self.shading_of(id.map(|i| (i, false)), id, shading);
        let sh = match &*sh {
            Ok(s) => s,
            Err(e) => {
                self.note(format!("shading not drawn: {e}"));
                return;
            }
        };
        let m = mul(&g.ctm, &self.base);
        let rect = format!(
            "x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            num(bbox.x0),
            num(bbox.y0),
            num(bbox.x1 - bbox.x0),
            num(bbox.y1 - bbox.y0)
        );
        let mut s = match (&self.color, sh.gradient()) {
            (Some(c), _) => format!("<rect {rect} fill=\"{c}\""),
            (None, Some(gr)) => {
                let Some(gid) = self.gradient_def(id, &gr, &m) else {
                    return;
                };
                format!("<rect {rect} fill=\"url(#{gid})\"")
            }
            (None, None) => {
                let Some(png) = self.shading_png(sh, &m, bbox, false) else {
                    return;
                };
                self.note("a shading is drawn as an image");
                self.sh.images += 1;
                format!(
                    "<image {rect} preserveAspectRatio=\"none\" xlink:href=\"data:image/png;base64,{png}\""
                )
            }
        };
        if g.fill_alpha < 1.0 {
            let _ = write!(s, " opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
        }
        self.blend(&mut s, g);
        s.push_str("/>");
        self.leaf(s, g, bbox);
    }

    fn begin_group(&mut self, group: &Group, g: &GState) {
        let mut attrs = String::new();
        if g.fill_alpha < 1.0 {
            let _ = write!(attrs, " opacity=\"{}\"", num(g.fill_alpha.max(0.0)));
        }
        let mut style = Vec::new();
        if g.blend != "Normal" && g.blend != "Compatible" {
            if let Some(b) = blend_mode(&g.blend) {
                style.push(format!("mix-blend-mode:{b}"));
            }
        }
        if group.isolated {
            style.push("isolation:isolate".into());
        }
        if !style.is_empty() {
            let _ = write!(attrs, " style=\"{}\"", style.join(";"));
        }
        if group.knockout {
            self.note("knockout groups are written as plain groups");
        }
        if !group.isolated && !attrs.is_empty() {
            self.note("non-isolated transparency groups with an opacity or blend mode are written isolated (as SVG groups with them are)");
        }
        if let Some(sm) = &g.smask {
            self.mask_def(sm);
        }
        let seq = self.open(
            Node {
                kind: NodeKind::Group,
                attrs,
                mask: g.smask.clone(),
                ..Node::default()
            },
            g,
        );
        self.groups.push(seq);
    }

    fn end_group(&mut self, _g: &GState) {
        if let Some(seq) = self.groups.pop() {
            self.close(seq);
        }
    }

    fn save(&mut self, g: &GState) {
        let seq = self.open(
            Node {
                kind: NodeKind::Save,
                ..Node::default()
            },
            g,
        );
        self.saves.push(seq);
    }

    fn restore(&mut self) {
        let floor = self.forms.last().map_or(0, |f| f.1);
        if self.saves.len() > floor {
            if let Some(seq) = self.saves.pop() {
                self.close(seq);
            }
        }
    }

    fn begin_text(&mut self, g: &GState) {
        self.texts += 1;
        let seq = self.open(
            Node {
                kind: NodeKind::Text,
                ..Node::default()
            },
            g,
        );
        self.text_group = Some(seq);
    }

    fn end_text(&mut self) {
        if let Some(seq) = self.text_group.take() {
            self.close(seq);
        }
    }

    fn begin_marked(&mut self, tag: &Name, props: Option<(Option<ObjId>, &Object)>, g: &GState) {
        let mut n = Node {
            kind: NodeKind::Marked,
            ..Node::default()
        };
        let d = props.and_then(|(_, o)| o.as_dict());
        if tag == "OC" {
            let file = self.sh.file;
            let name = d.and_then(|d| {
                if d.type_is("OCMD") {
                    return Some("OCMD".to_string());
                }
                let n = resolve(file, d.get_key("Name")?).1;
                n.as_string().map(|s| pdfi_model::text::decode(&s.bytes))
            });
            n.name = Some(name.unwrap_or_else(|| "Layer".into()));
            n.layer = true;
            n.keep = true;
            n.hidden = props
                .and_then(|(id, _)| id)
                .is_some_and(|id| self.sh.is_off(id));
        } else {
            let mut attrs = format!(" data-pdf-tag=\"{}\"", esc(&tag.as_str_lossy()));
            if let Some(mcid) = d.and_then(|d| d.get_i64("MCID")) {
                let _ = write!(attrs, " data-mcid=\"{mcid}\"");
            }
            // Kept only as a group of several elements.
            n.info = attrs;
        }
        let seq = self.open(n, g);
        self.marks.push(seq);
    }

    fn end_marked(&mut self) {
        let floor = self.forms.last().map_or(0, |f| f.2);
        if self.marks.len() > floor {
            if let Some(seq) = self.marks.pop() {
                self.close(seq);
            }
        }
    }

    fn begin_form(&mut self, _id: ObjId, name: &Name, g: &GState) {
        let seq = self.open(
            Node {
                kind: NodeKind::Form,
                name: Some(name.as_str_lossy().into_owned()),
                ..Node::default()
            },
            g,
        );
        self.forms.push((seq, self.saves.len(), self.marks.len()));
    }

    fn end_form(&mut self) {
        if let Some((seq, saves, marks)) = self.forms.pop() {
            self.saves.truncate(saves);
            self.marks.truncate(marks);
            self.close(seq);
        }
    }
}

// ------------------------------------------------------------- output

fn write_text(out: &mut String, t: &TextLeaf) {
    let _ = write!(
        out,
        "<text transform=\"{}\" xml:space=\"preserve\"{}",
        matrix_text(&t.matrix),
        t.attrs
    );
    let list = |v: &[f64]| v.iter().map(|&x| num(x)).collect::<Vec<_>>().join(" ");
    let _ = write!(out, " x=\"{}\"", list(&t.xs));
    if t.ys.windows(2).all(|w| w[0] == w[1]) {
        let _ = write!(out, " y=\"{}\"", num(t.ys.first().copied().unwrap_or(0.0)));
    } else {
        let _ = write!(out, " y=\"{}\"", list(&t.ys));
    }
    let _ = write!(out, ">{}</text>", esc(&t.chars));
}

/// Font attributes from `/BaseFont`: the PostScript name (without a
/// subset tag, or a Type 0 font's CMap) and the family it starts with, the
/// weight and style it names, a generic family.
fn font_attrs(font: &Font, size: f64) -> String {
    let mut name = font
        .base_font
        .as_deref()
        .map(|b| match b.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => {
                rest
            }
            _ => b,
        })
        .unwrap_or("");
    if let Some(cmap) = font
        .encoding
        .as_deref()
        .filter(|_| font.subtype.as_deref() == Some("Type0"))
    {
        name = name
            .strip_suffix(cmap)
            .and_then(|n| n.strip_suffix('-'))
            .unwrap_or(name);
    }
    let name = name.replace(['\'', '"', '\\'], "");
    let (family, style) = match name.rfind(['-', ',']) {
        Some(i) if style_words(&name[i + 1..]) => (&name[..i], &name[i + 1..]),
        _ => (name.as_str(), ""),
    };
    let flags = font.flags.unwrap_or(0);
    let generic = if flags & 1 != 0 {
        "monospace"
    } else if flags & 2 != 0 {
        "serif"
    } else {
        "sans-serif"
    };
    let mut families = Vec::new();
    for f in [name.as_str(), family] {
        if !f.is_empty() && !families.contains(&f) {
            families.push(f);
        }
    }
    let mut out = String::from(" font-family=\"");
    for f in families {
        let _ = write!(out, "'{}', ", esc(f));
    }
    let _ = write!(out, "{generic}\" font-size=\"{}\"", num(size));
    let s = style.to_ascii_lowercase();
    let weight = if let Some(w) = s
        .strip_prefix('w')
        .and_then(|w| w.parse::<u32>().ok())
        .filter(|w| (1..=9).contains(w))
    {
        w * 100
    } else if s.contains("black") || s.contains("heavy") {
        900
    } else if s.contains("extrabold") || s.contains("ultrabold") {
        800
    } else if s.contains("semibold") || s.contains("demi") {
        600
    } else if s.contains("bold") {
        700
    } else if s.contains("medium") {
        500
    } else if s.contains("extralight") || s.contains("ultralight") {
        200
    } else if s.contains("light") {
        300
    } else if s.contains("thin") {
        100
    } else {
        400
    };
    if weight != 400 {
        let _ = write!(out, " font-weight=\"{weight}\"");
    }
    if s.contains("italic") || s.contains("oblique") || flags & 64 != 0 {
        out.push_str(" font-style=\"italic\"");
    }
    out
}

/// Whether the end of a font name says a style (`Bold`, `W6`, ...).
fn style_words(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    [
        "regular",
        "roman",
        "book",
        "normal",
        "plain",
        "medium",
        "bold",
        "black",
        "heavy",
        "light",
        "thin",
        "demi",
        "italic",
        "oblique",
        "it",
        "semibold",
        "extrabold",
        "ultra",
    ]
    .iter()
    .any(|w| s.contains(w))
        || (s.len() == 2 && s.starts_with('w') && s[1..].parse::<u8>().is_ok())
}

fn blend_mode(n: &str) -> Option<&'static str> {
    Some(match n {
        "Multiply" => "multiply",
        "Screen" => "screen",
        "Overlay" => "overlay",
        "Darken" => "darken",
        "Lighten" => "lighten",
        "ColorDodge" => "color-dodge",
        "ColorBurn" => "color-burn",
        "HardLight" => "hard-light",
        "SoftLight" => "soft-light",
        "Difference" => "difference",
        "Exclusion" => "exclusion",
        "Hue" => "hue",
        "Saturation" => "saturation",
        "Color" => "color",
        "Luminosity" => "luminosity",
        _ => return None,
    })
}

/// Characters XML allows.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..)
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// A number with at most three decimals.
fn num(v: f64) -> String {
    num_with(v, 3)
}

fn num_with(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let mut s = format!("{v:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn matrix_text(m: &Matrix) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        num_with(m[0], 6),
        num_with(m[1], 6),
        num_with(m[2], 6),
        num_with(m[3], 6),
        num(m[4]),
        num(m[5])
    )
}

fn hex(rgb: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    format!("#{:02x}{:02x}{:02x}", q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.strip_prefix('#')?;
    let v = |i: usize| {
        u8::from_str_radix(s.get(i..i + 2)?, 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some([v(0)?, v(2)?, v(4)?])
}

fn url(id: String) -> String {
    format!("url(#{id})")
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

/// Rotation, reflection and uniform scaling only (strokes stay strokes
/// with a width of their own).
fn similarity(m: &Matrix) -> bool {
    let s = m[0].hypot(m[1]).max(m[2].hypot(m[3])).max(1e-12);
    let eps = 1e-6 * s;
    ((m[0] - m[3]).abs() < eps && (m[1] + m[2]).abs() < eps)
        || ((m[0] + m[3]).abs() < eps && (m[1] - m[2]).abs() < eps)
}

fn map_el(e: &PathEl, m: &Matrix) -> PathEl {
    let p = |x, y| apply(m, x, y);
    match *e {
        PathEl::Move(x, y) => {
            let (x, y) = p(x, y);
            PathEl::Move(x, y)
        }
        PathEl::Line(x, y) => {
            let (x, y) = p(x, y);
            PathEl::Line(x, y)
        }
        PathEl::Curve(a, b, c, d, e, f) => {
            let ((a, b), (c, d), (e, f)) = (p(a, b), p(c, d), p(e, f));
            PathEl::Curve(a, b, c, d, e, f)
        }
        PathEl::Close => PathEl::Close,
    }
}

/// Bounding box of a path (control points included).
fn path_box(els: &[PathEl]) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    let mut add = |x: f64, y: f64| {
        r = Some(union(
            r,
            Rect {
                x0: x,
                y0: y,
                x1: x,
                y1: y,
            },
        ))
    };
    for e in els {
        match *e {
            PathEl::Move(x, y) | PathEl::Line(x, y) => add(x, y),
            PathEl::Curve(a, b, c, d, e, f) => {
                add(a, b);
                add(c, d);
                add(e, f);
            }
            PathEl::Close => {}
        }
    }
    r
}

fn rect_els(r: &Rect) -> Vec<PathEl> {
    vec![
        PathEl::Move(r.x0, r.y0),
        PathEl::Line(r.x1, r.y0),
        PathEl::Line(r.x1, r.y1),
        PathEl::Line(r.x0, r.y1),
        PathEl::Close,
    ]
}

/// The rectangle a path is, when it is one with sides along the axes
/// (after `m`).
fn rect_of(els: &[PathEl], m: &Matrix) -> Option<Rect> {
    let mut pts = Vec::with_capacity(5);
    for (i, e) in els.iter().enumerate() {
        match *e {
            PathEl::Move(x, y) if i == 0 => pts.push(apply(m, x, y)),
            PathEl::Line(x, y) if i > 0 => pts.push(apply(m, x, y)),
            PathEl::Close if i + 1 == els.len() => {}
            _ => return None,
        }
    }
    if pts.len() == 5 {
        let (a, b) = (pts[0], pts[4]);
        if (a.0 - b.0).abs() > 1e-9 || (a.1 - b.1).abs() > 1e-9 {
            return None;
        }
        pts.pop();
    }
    if pts.len() != 4 {
        return None;
    }
    let flat = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 1e-6 || (p.1 - q.1).abs() < 1e-6;
    let alternating = (0..4).all(|i| {
        let (p, q, r) = (pts[i], pts[(i + 1) % 4], pts[(i + 2) % 4]);
        flat(p, q) && ((p.0 - q.0).abs() < 1e-6) != ((q.0 - r.0).abs() < 1e-6)
    });
    if !alternating {
        return None;
    }
    let mut r: Option<Rect> = None;
    for &(x, y) in &pts {
        r = Some(union(
            r,
            Rect {
                x0: x,
                y0: y,
                x1: x,
                y1: y,
            },
        ));
    }
    r.filter(|r| r.x1 > r.x0 && r.y1 > r.y0)
}

/// A path's element (tag and geometry, no paint), after `m`: `rect` for
/// axis-aligned rectangles, else `path`.
fn geometry(out: &mut String, els: &[PathEl], m: &Matrix) {
    if let Some(r) = rect_of(els, m) {
        let _ = write!(
            out,
            "rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            num(r.x0),
            num(r.y0),
            num(r.x1 - r.x0),
            num(r.y1 - r.y0)
        );
        return;
    }
    out.push_str("path d=\"");
    path_data(out, els, m);
    out.push('"');
}

/// SVG path data, after `m`.
fn path_data(out: &mut String, els: &[PathEl], m: &Matrix) {
    let p = |out: &mut String, x: f64, y: f64| {
        let (x, y) = apply(m, x, y);
        let _ = write!(out, "{} {}", num(x), num(y));
    };
    for e in els {
        match *e {
            PathEl::Move(x, y) => {
                out.push('M');
                p(out, x, y);
            }
            PathEl::Line(x, y) => {
                out.push('L');
                p(out, x, y);
            }
            PathEl::Curve(a, b, c, d, e, f) => {
                out.push('C');
                p(out, a, b);
                out.push(' ');
                p(out, c, d);
                out.push(' ');
                p(out, e, f);
            }
            PathEl::Close => out.push('Z'),
        }
    }
}

// ---------------------------------------------------------- encoding

/// A pixmap (premultiplied) as a PNG file: RGB when opaque, else RGBA.
fn png_of(p: &Pixmap) -> Vec<u8> {
    let (w, h) = (p.width() as usize, p.height() as usize);
    let opaque = p.pixels().iter().all(|c| c.alpha() == 255);
    let channels = if opaque { 3 } else { 4 };
    let mut rows = Vec::with_capacity(h * w * channels);
    for c in p.pixels() {
        let c = c.demultiply();
        rows.push(c.red());
        rows.push(c.green());
        rows.push(c.blue());
        if !opaque {
            rows.push(c.alpha());
        }
    }
    png(w as u32, h as u32, channels, &rows)
}

/// PNG of 8-bit RGB (3) or RGBA (4) samples, row by row.
fn png(w: u32, h: u32, channels: usize, data: &[u8]) -> Vec<u8> {
    let stride = w as usize * channels;
    let mut filtered = Vec::with_capacity((stride + 1) * h as usize);
    let zero = vec![0u8; stride];
    let mut line = vec![0u8; stride];
    for y in 0..h as usize {
        let row = &data[y * stride..][..stride];
        let up = if y > 0 {
            &data[(y - 1) * stride..][..stride]
        } else {
            &zero[..]
        };
        // The filter whose bytes sum smallest (as signed values).
        let mut best = (u64::MAX, 0u8);
        for f in 0..5u8 {
            filter(f, row, up, channels, &mut line);
            let cost = line
                .iter()
                .map(|&b| (b as i8).unsigned_abs() as u64)
                .sum::<u64>();
            if cost < best.0 {
                best = (cost, f);
            }
        }
        filter(best.1, row, up, channels, &mut line);
        filtered.push(best.1);
        filtered.extend_from_slice(&line);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &pdfi_syntax::write::deflate(&filtered));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn filter(f: u8, row: &[u8], up: &[u8], bpp: usize, out: &mut [u8]) {
    for i in 0..row.len() {
        let a = if i >= bpp { row[i - bpp] } else { 0 };
        let b = up[i];
        let c = if i >= bpp { up[i - bpp] } else { 0 };
        out[i] = row[i].wrapping_sub(match f {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((a as u16 + b as u16) / 2) as u8,
            _ => {
                let p = a as i16 + b as i16 - c as i16;
                let (pa, pb, pc) = (
                    (p - a as i16).abs(),
                    (p - b as i16).abs(),
                    (p - c as i16).abs(),
                );
                if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                }
            }
        });
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(2.50049), "2.5");
        assert_eq!(num(12.3456), "12.346");
    }

    #[test]
    fn base64_and_crc() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn rectangles() {
        let r = Rect {
            x0: 1.0,
            y0: 2.0,
            x1: 5.0,
            y1: 7.0,
        };
        assert_eq!(rect_of(&rect_els(&r), &IDENTITY), Some(r));
        let flip = [1.0, 0.0, 0.0, -1.0, 0.0, 10.0];
        assert_eq!(
            rect_of(&rect_els(&r), &flip),
            Some(Rect {
                x0: 1.0,
                y0: 3.0,
                x1: 5.0,
                y1: 8.0
            })
        );
        let skew = [1.0, 0.5, 0.0, 1.0, 0.0, 0.0];
        assert_eq!(rect_of(&rect_els(&r), &skew), None);
    }

    #[test]
    fn font_names() {
        let mut f = Font::unknown();
        f.base_font = Some("ABCDEF+Helvetica-BoldOblique".into());
        let a = font_attrs(&f, 12.0);
        assert!(
            a.contains("font-family=\"'Helvetica-BoldOblique', 'Helvetica', sans-serif\""),
            "{a}"
        );
        assert!(a.contains("font-weight=\"700\""), "{a}");
        assert!(a.contains("font-style=\"italic\""), "{a}");
        f.base_font = Some("MS-Gothic".into());
        let a = font_attrs(&f, 12.0);
        assert!(a.contains("'MS-Gothic', sans-serif"), "{a}");
        f.base_font = Some("HiraginoSans-W6".into());
        assert!(font_attrs(&f, 12.0).contains("font-weight=\"600\""));
        f.base_font = Some("Ryumin-Light-Identity-H".into());
        f.subtype = Some("Type0".into());
        f.encoding = Some("Identity-H".into());
        let a = font_attrs(&f, 12.0);
        assert!(a.contains("'Ryumin-Light', 'Ryumin'"), "{a}");
        assert!(a.contains("font-weight=\"300\""), "{a}");
    }
}
