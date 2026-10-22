//! Matrices and rectangles.

use pdfi_model::Rect;

/// `[a b c d e f]`: `x' = a x + c y + e`, `y' = b x + d y + f`.
pub type Matrix = [f64; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n` (PDF writes this `m × n`).
pub fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

pub fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

pub fn translate(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// How much `m` scales lengths, on average.
pub fn scale_of(m: &Matrix) -> f64 {
    (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
}

pub fn point(x: f64, y: f64) -> Rect {
    Rect {
        x0: x,
        y0: y,
        x1: x,
        y1: y,
    }
}

pub fn add_point(r: &mut Option<Rect>, x: f64, y: f64) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    *r = Some(match *r {
        None => point(x, y),
        Some(r) => Rect {
            x0: r.x0.min(x),
            y0: r.y0.min(y),
            x1: r.x1.max(x),
            y1: r.y1.max(y),
        },
    });
}

pub fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => Rect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        },
    }
}

/// The overlap of `a` and `b`, `None` when they do not overlap.
pub fn intersect(a: &Rect, b: &Rect) -> Option<Rect> {
    let r = Rect {
        x0: a.x0.max(b.x0),
        y0: a.y0.max(b.y0),
        x1: a.x1.min(b.x1),
        y1: a.y1.min(b.y1),
    };
    (r.x0 <= r.x1 && r.y0 <= r.y1).then_some(r)
}

/// Bounding box of `r` transformed by `m`.
pub fn transform(r: &Rect, m: &Matrix) -> Rect {
    let mut out = None;
    for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)] {
        let (x, y) = apply(m, x, y);
        add_point(&mut out, x, y);
    }
    out.unwrap_or(*r)
}

pub fn grow(r: &Rect, d: f64) -> Rect {
    Rect {
        x0: r.x0 - d,
        y0: r.y0 - d,
        x1: r.x1 + d,
        y1: r.y1 + d,
    }
}

pub fn unit_square() -> Rect {
    Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_order() {
        // Scale by 2, then translate by (10, 0).
        let m = mul(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0], &translate(10.0, 0.0));
        assert_eq!(apply(&m, 1.0, 1.0), (12.0, 2.0));
    }
}
