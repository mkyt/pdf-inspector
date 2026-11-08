import Foundation
import Testing

@testable import PdfInspectorKit

@Suite struct JavaScriptTests {
    /// Each highlighted span as its text and kind.
    private func spans(_ text: String) -> [String] {
        let ns = text as NSString
        return highlightJavaScript(text).map { "\(ns.substring(with: $0.range)):\($0.token)" }
    }

    @Test func keywordsCallsAndMembers() {
        #expect(spans("var f = this.getField(\"Total\"); if (f.value > 0x1F) app.alert(1.5e3);") == [
            "var:keyword", "this:literal", "getField:function", "\"Total\":string", "if:keyword",
            "value:property", "0x1F:number", "alert:function", "1.5e3:number",
        ])
    }

    @Test func commentsAndStringsHideCode() {
        #expect(spans("// if (x)\n/* var */ 'it\\'s' \"a // b\"") == [
            "// if (x):comment", "/* var */:comment", "'it\\'s':string", "\"a // b\":string",
        ])
    }

    @Test func regexpOrDivision() {
        #expect(spans("x = a / b / c;") == [])
        #expect(spans("s.replace(/[/]+/g, '')") == ["replace:function", "/[/]+/g:regexp", "'':string"])
        #expect(spans("return /a/.test(s)") == ["return:keyword", "/a/:regexp", "test:function"])
        #expect(spans("(a) / 2") == ["2:number"])
    }

    @Test func templateLiterals() {
        #expect(spans("`n=${n + 1} {}`") == ["`n=${:string", "1:number", "} {}`:string"])
        #expect(spans("`a${ {b: 1} }c`") == ["`a${:string", "1:number", "}c`:string"])
    }

    @Test func contextualKeywordsAsNames() {
        #expect(spans("m.get(k); get(k); let of = 1") == [
            "get:function", "get:function", "let:keyword", "of:keyword", "1:number",
        ])
    }

    @Test func offsetsAreUTF16() {
        let r = highlightJavaScript("'é😀' + 1")
        #expect(r.map(\.range) == [NSRange(location: 0, length: 5), NSRange(location: 8, length: 1)])
    }
}
