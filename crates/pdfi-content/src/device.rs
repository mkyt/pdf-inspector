//! What the interpreter hands to a renderer: paths, glyph runs and images
//! with the graphics state they are painted in.
//!
//! The interpreter only records geometry while it has a [`Device`]
//! (`Rec::draw`); the analysis pass keeps bounding boxes alone.

use std::sync::Arc;

use pdfi_model::Rect;
use pdfi_syntax::{Dict, ObjId, Object, Stream};

use crate::geom::Matrix;
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
    pub code: u32,
    /// Origin in text space (before the text matrix), font size and
    /// horizontal scaling applied.
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

    /// Text in render modes that paint (not 3 / 7; clipping modes are
    /// painted without the clip).
    fn text(&mut self, run: &GlyphRun<'_>, g: &GState);

    /// `res`: the resources of the content (inline images name color
    /// spaces through them).
    fn image(&mut self, image: ImageData<'_>, res: &Resources, g: &GState);

    /// `sh`: fill the current clip with a shading.
    fn shading(&mut self, _shading: &Object, _g: &GState) {}
}
