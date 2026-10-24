import Foundation

// Text for what the views show (`apps/web/src/format.ts`).

extension ObjRef {
    /// `12 0 R`
    public var refText: String { "\(num) \(generation) R" }
    /// `12 0 obj`
    public var objText: String { "\(num) \(generation) obj" }
}

public func bytesText(_ n: UInt64) -> String {
    if n < 1024 { return "\(n) B" }
    let units = ["KiB", "MiB", "GiB"]
    var v = Double(n) / 1024
    var i = 0
    while v >= 1024 && i < units.count - 1 {
        v /= 1024
        i += 1
    }
    return String(format: v < 10 ? "%.1f %@" : "%.0f %@", v, units[i])
}

/// `3` or `iii (3)`: 1-based, with the page label when it differs.
public func pageText(_ index: UInt32, label: String?) -> String {
    let n = String(index + 1)
    guard let label, label != n else { return n }
    return "\(label) (\(n))"
}

extension ByteSpan {
    public var text: String {
        "\(start.formatted())–\(end.formatted()) (\((end - start).formatted()) B)"
    }
}

extension XrefEntry {
    public var text: String {
        switch self {
        case .free(let next, _): "free (next \(next))"
        case .inUse(let offset, _): "@\(offset.formatted())"
        case .compressed(let stream, let index): "in \(stream) [\(index)]"
        case .edited: "edited (not saved)"
        }
    }
}

extension Value {
    /// Short inline rendering, close to PDF syntax.
    public var text: String {
        switch self {
        case .null: "null"
        case .bool(let v): String(v)
        case .integer(let v): String(v)
        case .real(let v): String(v)
        case .string(_, _, let text, let hex, let truncated):
            text.contains(where: { $0.isPrintable == false })
                ? "<\(hex)\(truncated ? "…" : "")>" : "(\(text)\(truncated ? "…" : ""))"
        case .name(let name): "/\(name)"
        case .array(let len): "[ \(len) items ]"
        case .dict(let len, let typeName): "<< \(len) keys >>" + (typeName.map { " /\($0)" } ?? "")
        case .stream(let len, let typeName, let rawLen):
            "stream << \(len) keys >>" + (typeName.map { " /\($0)" } ?? "") + " · \(bytesText(rawLen))"
        case .ref(let id): id.refText
        }
    }
}

extension Character {
    fileprivate var isPrintable: Bool {
        !(unicodeScalars.first.map { $0.value < 0x20 && $0 != "\t" && $0 != "\n" && $0 != "\r" }
            ?? false) && self != "\u{FFFD}"
    }
}

extension Value {
    /// A container with something in it.
    public var hasChildren: Bool {
        switch self {
        case .array(let len), .dict(let len, _), .stream(let len, _, _): len > 0
        default: false
        }
    }
}

private let hexDigits = Array("0123456789abcdef")

/// `0a`
public func hexByte(_ b: UInt8) -> String {
    String([hexDigits[Int(b >> 4)], hexDigits[Int(b & 15)]])
}

/// The byte as a printable ASCII character, `·` otherwise.
public func asciiByte(_ b: UInt8) -> Character {
    b >= 0x20 && b < 0x7f ? Character(UnicodeScalar(b)) : "·"
}

/// Bytes as text: Latin-1 with control characters (except whitespace)
/// shown as `·`.
public func bytesAsText<D: Collection<UInt8>>(_ data: D) -> String {
    var s = String.UnicodeScalarView()
    s.reserveCapacity(data.count)
    for b in data {
        s.append(UnicodeScalar((b < 0x20 && b != 9 && b != 10 && b != 13) || b == 0x7f ? 0xb7 : b))
    }
    return String(s)
}

/// Each byte as the character with that code (Latin-1), so any data
/// round-trips through text.
public func latin1<D: Collection<UInt8>>(_ data: D) -> String {
    var s = String.UnicodeScalarView()
    s.reserveCapacity(data.count)
    for b in data { s.append(UnicodeScalar(b)) }
    return String(s)
}

/// The inverse of `latin1`; `nil` if a character is beyond U+00FF.
public func fromLatin1(_ s: String) -> Data? {
    var out = Data(capacity: s.unicodeScalars.count)
    for c in s.unicodeScalars {
        guard c.value <= 0xff else { return nil }
        out.append(UInt8(c.value))
    }
    return out
}

extension Rect {
    /// `[0 0 612 792]`
    public var text: String { "[\(numText(x0)) \(numText(y0)) \(numText(x1)) \(numText(y1))]" }
    /// `612 × 792 pt`
    public var sizeText: String { "\(numText(x1 - x0)) × \(numText(y1 - y0)) pt" }
}

/// A number without trailing zeros, at most 2 decimals.
public func numText(_ n: Double) -> String {
    if n == n.rounded() && abs(n) < 1e15 { return String(Int64(n)) }
    var s = String(format: "%.2f", n)
    while s.hasSuffix("0") { s.removeLast() }
    if s.hasSuffix(".") { s.removeLast() }
    return s
}

extension AttrOrigin {
    public var text: String {
        switch self {
        case .page: "on the page"
        case .inherited(let from): "inherited from \(from.objText)"
        case .default(let of): of == "0" ? "default" : "default (= \(of))"
        case .missing: "missing"
        }
    }
}

extension Destination {
    public var text: String {
        let whereText =
            if let p = pageIndex { "page \(p + 1)" } else if let r = remotePage {
                "page \(r + 1) of another file"
            } else { error ?? "?" }
        let fitText = fit.map { f in " /\(f)" + params.map { " \($0.map(numText) ?? "null")" }.joined() } ?? ""
        return (named.map { "“\($0)” → " } ?? "") + whereText + fitText
    }
}

extension LinkTarget {
    public var text: String {
        switch self {
        case .dest(let d): d.text
        case .uri(let u): u
        case .action(let a, let detail): detail.map { "\(a): \($0)" } ?? a
        }
    }
}

extension NodeRoot {
    /// `trailer` or `12 0 obj`
    public var text: String {
        switch self {
        case .trailer: "trailer"
        case .object(let id): id.objText
        }
    }

    public var target: Target {
        switch self {
        case .trailer: .trailer
        case .object(let id): .object(id)
        }
    }
}

extension Severity {
    /// For sorting: errors first.
    public var rank: Int {
        switch self {
        case .info: 0
        case .warning: 1
        case .error: 2
        }
    }
}

extension [DiagnosticInfo] {
    /// The most severe, if any.
    public var worst: Severity? { map(\.severity).max { $0.rank < $1.rank } }
}

/// `1-3, 7` (1-based) as 0-based page indices; `nil` if malformed or out
/// of `1...count`.
public func parsePages(_ s: String, count: UInt32) -> [UInt32]? {
    var out: [UInt32] = []
    for part in s.split(separator: ",").map({ $0.trimmingCharacters(in: .whitespaces) }) where !part.isEmpty {
        let ends = part.split(separator: "-", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }
        guard ends.count <= 2, let a = UInt32(ends[0]) else { return nil }
        let b = ends.count == 2 ? UInt32(ends[1]) : a
        guard let b, a >= 1, b >= a, b <= count else { return nil }
        out += (a...b).map { $0 - 1 }
    }
    return out.isEmpty ? nil : out
}
