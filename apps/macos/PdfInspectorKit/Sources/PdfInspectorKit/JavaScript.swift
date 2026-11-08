import Foundation

/// What a span of JavaScript is, for highlighting.
public enum JSToken: Equatable, Sendable {
    case keyword, literal, number, string, regexp, comment, function, property
}

/// Spans of JavaScript to highlight, as UTF-16 offsets (`NSRange`s of the
/// text). A tokenizer rather than a parser: it follows strings, template
/// literals (with `${}`), comments and regular expressions so that what is
/// inside them is not taken for code, and tells a `/` that starts a regular
/// expression from a division by the token before it. Plain identifiers,
/// operators and punctuation get no span.
public func highlightJavaScript(_ text: String) -> [(range: NSRange, token: JSToken)] {
    var s = JSScanner(Array(text.utf16))
    s.run()
    return s.out
}

private let keywords = Set([
    "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do",
    "else", "export", "extends", "finally", "for", "function", "if", "import", "in", "instanceof",
    "new", "return", "super", "switch", "throw", "try", "typeof", "var", "void", "while", "with",
]).union(contextual)

private let literals: Set<String> = ["true", "false", "null", "undefined", "this", "NaN", "Infinity"]

/// Keywords only in some places: elsewhere (`obj.get(k)`, `set(v)`) names.
private let contextual: Set<String> = ["async", "await", "of", "static", "get", "set", "yield", "let"]

private struct JSScanner {
    let c: [UInt16]
    var i = 0
    var out: [(range: NSRange, token: JSToken)] = []
    /// Whether a `/` here would start a regular expression.
    var regexOK = true
    /// Open braces of each `${` being scanned, innermost last.
    var templates: [Int] = []

    init(_ c: [UInt16]) { self.c = c }

    func at(_ k: Int) -> UInt16 { k < c.count ? c[k] : 0 }

    mutating func emit(_ from: Int, _ token: JSToken) {
        if i > from { out.append((NSRange(location: from, length: i - from), token)) }
    }

    mutating func run() {
        while i < c.count {
            let x = c[i]
            let start = i
            switch x {
            case 0x20, 0x09, 0x0a, 0x0d, 0x0b, 0x0c, 0xa0, 0xfeff, 0x2028, 0x2029:
                i += 1
            case ch(".") where isDigit(at(i + 1)), _ where isDigit(x):
                number()
                emit(start, .number)
                regexOK = false
            case ch("\""), ch("'"):
                quoted(x)
                emit(start, .string)
                regexOK = false
            case ch("`"):
                i += 1
                template(from: start)
                regexOK = false
            case ch("/") where at(i + 1) == ch("/"):
                while i < c.count && !isLineEnd(c[i]) { i += 1 }
                emit(start, .comment)
            case ch("/") where at(i + 1) == ch("*"):
                i += 2
                while i < c.count && !(c[i] == ch("*") && at(i + 1) == ch("/")) { i += 1 }
                i = min(i + 2, c.count)
                emit(start, .comment)
            case ch("/") where regexOK:
                regexp()
                emit(start, .regexp)
                regexOK = false
            case ch("{"):
                i += 1
                if let n = templates.last { templates[templates.count - 1] = n + 1 }
                regexOK = true
            case ch("}"):
                i += 1
                if let n = templates.last {
                    if n == 0 {
                        // The end of a `${}`: back in the template literal.
                        templates.removeLast()
                        template(from: start)
                    } else {
                        templates[templates.count - 1] = n - 1
                    }
                }
                regexOK = false
            case ch(")"), ch("]"):
                i += 1
                regexOK = false
            case _ where isIdentStart(x):
                word()
            default:
                i += 1
                // `++` and `--` end an operand as often as they start one;
                // a division after them is the likelier reading.
                regexOK = !((x == ch("+") || x == ch("-")) && at(i - 2) == x)
            }
        }
    }

    mutating func word() {
        let start = i
        while i < c.count && isIdentPart(c[i]) { i += 1 }
        let w = String(utf16CodeUnits: Array(c[start..<i]), count: i - start)
        // A member name after `.` is a property, whatever it is spelled as.
        var p = start - 1
        while p >= 0 && isSpace(c[p]) { p -= 1 }
        let member = p >= 0 && c[p] == ch(".") && !(p >= 1 && c[p - 1] == ch("."))
        var n = i
        while n < c.count && isSpace(c[n]) { n += 1 }
        let call = at(n) == ch("(")
        if !member && keywords.contains(w) && !(call && contextual.contains(w)) {
            emit(start, .keyword)
            // After a keyword an expression starts (`return /x/`), except
            // after `super`, which is one.
            regexOK = w != "super"
            return
        }
        if !member && literals.contains(w) {
            emit(start, .literal)
        } else if call {
            emit(start, .function)
        } else if member {
            emit(start, .property)
        }
        regexOK = false
    }

    mutating func number() {
        if c[i] == ch("0"), let x = at(i + 1).ascii, "xXoObB".contains(x) {
            i += 2
            while i < c.count && (isHex(c[i]) || c[i] == ch("_")) { i += 1 }
        } else {
            while i < c.count && (isDigit(c[i]) || c[i] == ch("_")) { i += 1 }
            if at(i) == ch(".") {
                i += 1
                while i < c.count && (isDigit(c[i]) || c[i] == ch("_")) { i += 1 }
            }
            if at(i) == ch("e") || at(i) == ch("E") {
                var k = i + 1
                if at(k) == ch("+") || at(k) == ch("-") { k += 1 }
                if isDigit(at(k)) {
                    i = k
                    while i < c.count && isDigit(c[i]) { i += 1 }
                }
            }
        }
        if at(i) == ch("n") { i += 1 }
    }

    /// A string in `q` quotes; an unclosed one ends with its line.
    mutating func quoted(_ q: UInt16) {
        i += 1
        while i < c.count {
            let x = c[i]
            if x == ch("\\") {
                i += 2
                continue
            }
            if x == q { i += 1; return }
            if isLineEnd(x) { return }
            i += 1
        }
        i = min(i, c.count)
    }

    /// The rest of a template literal from `from`, up to its end or to a `${`.
    mutating func template(from: Int) {
        while i < c.count {
            let x = c[i]
            if x == ch("\\") {
                i += 2
                continue
            }
            if x == ch("`") {
                i += 1
                break
            }
            if x == ch("$") && at(i + 1) == ch("{") {
                i += 2
                emit(from, .string)
                templates.append(0)
                regexOK = true
                return
            }
            i += 1
        }
        i = min(i, c.count)
        emit(from, .string)
    }

    mutating func regexp() {
        i += 1
        var inClass = false
        while i < c.count {
            let x = c[i]
            if isLineEnd(x) { return }
            if x == ch("\\") {
                i += 2
                continue
            }
            if x == ch("[") { inClass = true }
            if x == ch("]") { inClass = false }
            if x == ch("/") && !inClass {
                i += 1
                while i < c.count && isIdentPart(c[i]) { i += 1 }
                return
            }
            i += 1
        }
    }
}

private func ch(_ s: Unicode.Scalar) -> UInt16 { UInt16(s.value) }
private func isDigit(_ x: UInt16) -> Bool { x >= 0x30 && x <= 0x39 }
private func isHex(_ x: UInt16) -> Bool { isDigit(x) || (x | 0x20 >= 0x61 && x | 0x20 <= 0x66) }
private func isLineEnd(_ x: UInt16) -> Bool { x == 0x0a || x == 0x0d || x == 0x2028 || x == 0x2029 }
private func isSpace(_ x: UInt16) -> Bool { x == 0x20 || x == 0x09 || isLineEnd(x) }
private func isIdentStart(_ x: UInt16) -> Bool {
    (x | 0x20 >= 0x61 && x | 0x20 <= 0x7a) || x == ch("_") || x == ch("$") || x == ch("\\") || x >= 0x80
}
private func isIdentPart(_ x: UInt16) -> Bool { isIdentStart(x) || isDigit(x) }

private extension UInt16 {
    var ascii: Character? { self < 0x80 ? Character(Unicode.Scalar(UInt8(self))) : nil }
}
