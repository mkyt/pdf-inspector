//! Helpers for reading values that may be indirect.

use std::ops::Deref;
use std::sync::Arc;

use pdfi_syntax::{DictExt, IndirectObject, Name, ObjId, Object, PdfFile};

static NULL: Object = Object::Null;

/// A resolved value: borrowed when it was direct, held otherwise.
pub enum Res<'a> {
    Direct(&'a Object),
    Held(Arc<IndirectObject>),
}

impl Deref for Res<'_> {
    type Target = Object;

    fn deref(&self) -> &Object {
        match self {
            Res::Direct(o) => o,
            Res::Held(h) => &h.node.obj,
        }
    }
}

/// Follow references; missing or unreadable objects are null. Also returns
/// the id of the (last) referenced object.
pub fn resolve<'a>(file: &PdfFile, o: &'a Object) -> (Option<ObjId>, Res<'a>) {
    let Object::Ref(mut id) = *o else {
        return (None, Res::Direct(o));
    };
    for _ in 0..8 {
        match file.get(id) {
            Ok(h) => match h.node.obj {
                Object::Ref(next) => id = next,
                _ => return (Some(id), Res::Held(h)),
            },
            Err(_) => break,
        }
    }
    (Some(id), Res::Direct(&NULL))
}

/// Value of `key` in `d`, resolved.
pub fn get<'a>(file: &PdfFile, d: &'a pdfi_syntax::Dict, key: &str) -> Option<Res<'a>> {
    d.get_key(key).map(|o| resolve(file, o).1)
}

pub fn number(file: &PdfFile, o: &Object) -> Option<f64> {
    resolve(file, o).1.as_f64()
}

pub fn int(file: &PdfFile, d: &pdfi_syntax::Dict, key: &str) -> Option<i64> {
    get(file, d, key).and_then(|v| v.as_i64())
}

pub fn name(file: &PdfFile, d: &pdfi_syntax::Dict, key: &str) -> Option<Name> {
    get(file, d, key).and_then(|v| v.as_name().cloned())
}

pub fn name_str(n: &Name) -> String {
    n.as_str_lossy().into_owned()
}

/// Text string at `key`, decoded.
pub fn text(file: &PdfFile, d: &pdfi_syntax::Dict, key: &str) -> Option<String> {
    get(file, d, key).and_then(|v| v.as_string().map(|s| crate::text::decode(&s.bytes)))
}

/// A rectangle, normalized so that `x0 <= x1` and `y0 <= y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn read(file: &PdfFile, o: &Object) -> Option<Rect> {
        let r = resolve(file, o).1;
        let a = r.as_array()?;
        if a.len() != 4 {
            return None;
        }
        let mut v = [0.0; 4];
        for (x, n) in v.iter_mut().zip(a) {
            *x = number(file, &n.obj)?;
        }
        Some(Rect {
            x0: v[0].min(v[2]),
            y0: v[1].min(v[3]),
            x1: v[0].max(v[2]),
            y1: v[1].max(v[3]),
        })
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x0.max(o.x0);
        let y0 = self.y0.max(o.y0);
        Rect {
            x0,
            y0,
            x1: self.x1.min(o.x1).max(x0),
            y1: self.y1.min(o.y1).max(y0),
        }
    }
}
