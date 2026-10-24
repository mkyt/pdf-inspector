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
