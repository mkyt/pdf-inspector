import CoreGraphics
import Foundation

/// Folded blocks of a content's operations: opener → last operation (the
/// web `OpEditor` folds).
public typealias Folds = [UInt32: UInt32]

/// Rows of an operation list with some blocks folded: which operation a row
/// shows and which row shows an operation.
public struct OpLayout: Equatable, Sendable {
    private struct Hidden: Equatable {
        /// Hidden operations `a...b`.
        let a: Int
        let b: Int
        /// Operations hidden before `a`.
        let before: Int
    }

    private var hidden: [Hidden] = []
    public let rows: Int

    public init(folds: Folds, total: Int) {
        var count = 0
        var last = -1
        for (o, e) in folds.map({ (Int($0.key), Int($0.value)) }).sorted(by: { $0.0 < $1.0 }) {
            let a = o + 1
            let b = min(e, total - 1)
            if o <= last || b < a { continue }
            hidden.append(Hidden(a: a, b: b, before: count))
            count += b - a + 1
            last = b
        }
        rows = total - count
    }

    /// The last interval starting at or before `x` by `key`.
    private func find(_ x: Int, _ key: (Hidden) -> Int) -> Hidden? {
        var lo = 0
        var hi = hidden.count - 1
        var best: Hidden?
        while lo <= hi {
            let mid = (lo + hi) / 2
            if key(hidden[mid]) <= x {
                best = hidden[mid]
                lo = mid + 1
            } else {
                hi = mid - 1
            }
        }
        return best
    }

    /// The operation row `row` shows.
    public func op(_ row: Int) -> Int {
        guard let h = find(row, { $0.a - $0.before }) else { return row }
        return row + h.before + (h.b - h.a + 1)
    }

    /// The row showing `op` (its folded opener's when hidden).
    public func row(_ op: Int) -> Int {
        guard let h = find(op, { $0.a }) else { return op }
        if op <= h.b { return h.a - 1 - h.before }
        return op - h.before - (h.b - h.a + 1)
    }
}

/// Folds after an edit that put `count` operations in place of `from..<to`.
public func remapFolds(_ folds: Folds, from: UInt32, to: UInt32, count: UInt32) -> Folds {
    let d = Int64(count) - Int64(to - from)
    var out = Folds()
    for (o, e) in folds {
        if e < from {
            out[o] = e
        } else if o >= to {
            out[UInt32(Int64(o) + d)] = UInt32(Int64(e) + d)
        } else if o < from && e >= to {
            out[o] = UInt32(Int64(e) + d)
        }
    }
    return out
}

/// Folds after moving operations `from..<to` before `before`; a block that
/// does not stay in one piece unfolds.
public func moveFolds(_ folds: Folds, from: UInt32, to: UInt32, before: UInt32) -> Folds {
    let n = to - from
    let at = before > from ? before - n : before
    func map(_ i: UInt32) -> UInt32 {
        if i >= from && i < to { return at + (i - from) }
        let rest = i >= to ? i - n : i
        return rest >= at ? rest + n : rest
    }
    var out = Folds()
    for (o, e) in folds {
        let a = map(o)
        let b = map(e)
        if Int64(b) - Int64(a) == Int64(e) - Int64(o) { out[a] = b }
    }
    return out
}

extension ViewInfo {
    /// The content's space to the view (points from the top left, y down).
    public var transform: CGAffineTransform {
        let m = matrix.count == 6 ? matrix : [1, 0, 0, 1, 0, 0]
        return CGAffineTransform(a: m[0], b: m[1], c: m[2], d: m[3], tx: m[4], ty: m[5])
    }

    /// A rectangle of the content's space in view points.
    public func toView(_ r: Rect) -> CGRect {
        CGRect(x: r.x0, y: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0).applying(transform)
    }

    /// A rectangle of the view in the content's space.
    public func toContent(_ r: CGRect) -> Rect {
        let c = r.applying(transform.inverted())
        return Rect(x0: c.minX, y0: c.minY, x1: c.maxX, y1: c.maxY)
    }

    /// A view point in the content's space.
    public func toContent(_ p: CGPoint) -> CGPoint {
        p.applying(transform.inverted())
    }
}
