//! What the interpreter hands to a renderer: paths, glyph runs and images
//! with the graphics state they are painted in.
//!
//! The interpreter only records geometry while it has a [`Device`]
//! (`Rec::draw`); the analysis pass keeps bounding boxes alone.

use std::sync::Arc;

use pdfi_font::{Code, Seg};
use pdfi_model::Rect;
use pdfi_syntax::{Dict, Name, ObjId, Object, PdfFile, Stream};

use crate::geom::{Matrix, apply, mul};
use crate::interp::{FontRef, GState, Resources};

/// One element of a path, in the content's space (the CTM applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathEl {
    Move(f64, f64),
    Line(f64, f64),
    Curve(f64, f64, f64, f64, f64, f64),
    Close,
}

/// A clipping path and the ones it is intersected with (what `q` saves).
#[derive(Debug)]
pub struct ClipPath {
    pub path: Vec<PathEl>,
    pub even_odd: bool,
    pub parent: Option<Arc<ClipPath>>,
}

// Contents can nest clips without bound: drop long chains without recursing.
impl Drop for ClipPath {
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(p) = next {
            next = match Arc::try_unwrap(p) {
                Ok(mut c) => c.parent.take(),
                Err(_) => None,
            };
        }
    }
}

/// One character shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub code: Code,
    /// Where the glyph's (horizontal) origin is, in text space (before the
    /// text matrix), font size and horizontal scaling applied.
    pub x: f64,
    pub y: f64,
    /// The box the bounding boxes use (advance × ascent / descent).
    pub bbox: Rect,
}

/// The characters of one text-showing operation.
#[derive(Debug)]
pub struct GlyphRun<'a> {
    pub font: &'a FontRef,
    /// Text space to the content's space (text matrix × CTM).
    pub matrix: Matrix,
    pub size: f64,
    /// Horizontal scaling (1 = 100 %).
    pub scale: f64,
    pub rise: f64,
    pub glyphs: Vec<Glyph>,
}

impl GlyphRun<'_> {
    /// Glyph outline space (text space for a font size of 1) to the
    /// content's space.
    pub fn glyph_matrix(&self, g: &Glyph) -> Matrix {
        mul(
            &[self.size * self.scale, 0.0, 0.0, self.size, g.x, g.y],
            &self.matrix,
        )
    }

    /// Append the outline of a glyph in the content's space; its box when
    /// the font has no outlines. Returns whether there was an outline.
    pub fn glyph_path(&self, file: &PdfFile, g: &Glyph, out: &mut Vec<PathEl>) -> bool {
        let font = &self.font.info;
        if !font.has_outlines(file) {
            let b = &g.bbox;
            let p = |x, y| apply(&self.matrix, x, y);
            let [a, bb, c, d] = [p(b.x0, b.y0), p(b.x1, b.y0), p(b.x1, b.y1), p(b.x0, b.y1)];
            out.extend([
                PathEl::Move(a.0, a.1),
                PathEl::Line(bb.0, bb.1),
                PathEl::Line(c.0, c.1),
                PathEl::Line(d.0, d.1),
                PathEl::Close,
            ]);
            return false;
        }
        let Some(o) = font.outline(file, g.code) else {
            return false;
        };
        let m = self.glyph_matrix(g);
        let p = |x: f32, y: f32| apply(&m, x.into(), y.into());
        out.reserve(o.segs.len());
        for s in &o.segs {
            out.push(match *s {
                Seg::Move(x, y) => {
                    let (x, y) = p(x, y);
                    PathEl::Move(x, y)
                }
                Seg::Line(x, y) => {
                    let (x, y) = p(x, y);
                    PathEl::Line(x, y)
                }
                Seg::Curve(a, b, c, d, e, f) => {
                    let ((a, b), (c, d), (e, f)) = (p(a, b), p(c, d), p(e, f));
                    PathEl::Curve(a, b, c, d, e, f)
                }
                Seg::Close => PathEl::Close,
            });
        }
        true
    }
}

/// A soft mask set by `gs` (`/SMask` of an `/ExtGState`).
#[derive(Debug, PartialEq)]
pub struct SoftMask {
    /// `/S /Luminosity` (else `/Alpha`).
    pub luminosity: bool,
    /// The transparency group (a form XObject) and its object.
    pub id: ObjId,
    pub group: Arc<Object>,
    /// `/BC`: the backdrop color, in the group's color space.
    pub backdrop: Option<Vec<f64>>,
    /// `/TR`: the transfer function (`None`: identity).
    pub transfer: Option<Object>,
    /// The CTM when the mask was set (the group is painted in it).
    pub ctm: Matrix,
}

/// A transparency group (a form XObject with `/Group << /S /Transparency >>`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub isolated: bool,
    pub knockout: bool,
}

/// An image to paint in the unit square of user space.
#[derive(Clone, Copy, Debug)]
pub enum ImageData<'a> {
    XObject {
        id: Option<ObjId>,
        stream: &'a Stream,
    },
    /// `BI … ID … EI`: entries as written (abbreviations), and the data.
    Inline { dict: &'a Dict, data: &'a [u8] },
}

/// Receives what a content paints, in painting order.
pub trait Device {
    /// The top-level operation that the following calls come from (forms
    /// paint under the `Do` that calls them).
    fn op(&mut self, _index: u32) {}

    fn fill(&mut self, path: &[PathEl], even_odd: bool, g: &GState);

    fn stroke(&mut self, path: &[PathEl], g: &GState);

    /// Text in render modes that paint (not 3 / 7; the clipping modes
    /// clip after `ET`). Type 3 glyphs come as the paths and images of
    /// their procedures instead.
    fn text(&mut self, run: &GlyphRun<'_>, g: &GState);

    /// `res`: the resources of the content (inline images name color
    /// spaces through them).
    fn image(&mut self, image: ImageData<'_>, res: &Resources, g: &GState);

    /// `sh`: fill the current clip with a shading (`id`: its object, for
    /// the data of mesh shadings).
    fn shading(&mut self, _id: Option<ObjId>, _shading: &Object, _g: &GState) {}

    /// A transparency group starts: what follows (until
    /// [`Device::end_group`]) is painted into the group, which is then
    /// composited with `g`'s alpha, blend mode, soft mask and clip.
    fn begin_group(&mut self, _group: &Group, _g: &GState) {}

    fn end_group(&mut self, _g: &GState) {}

    // How what is painted is put together, for devices that keep the
    // structure (an SVG export): saved states, text objects, marked
    // content and forms. Each opening call is matched by its closing one
    // within the same content (forms close what they leave open).

    /// `q` (in state `g`).
    fn save(&mut self, _g: &GState) {}

    /// `Q` restoring the state of a `q`.
    fn restore(&mut self) {}

    /// `BT`.
    fn begin_text(&mut self, _g: &GState) {}

    /// `ET`.
    fn end_text(&mut self) {}

    /// `BMC` / `BDC`: the tag, and the properties (inline, or the
    /// `/Properties` resource and its object).
    fn begin_marked(&mut self, _tag: &Name, _props: Option<(Option<ObjId>, &Object)>, _g: &GState) {
    }

    /// `EMC` closing a `BMC` / `BDC`.
    fn end_marked(&mut self) {}

    /// A form XObject (resource `name`) starts painting, in state `g`
    /// (before its matrix and `/BBox` apply).
    fn begin_form(&mut self, _id: ObjId, _name: &Name, _g: &GState) {}

    fn end_form(&mut self) {}
}
