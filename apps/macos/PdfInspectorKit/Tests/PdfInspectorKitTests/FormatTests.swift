import CoreGraphics
import Foundation
import Testing

@testable import PdfInspectorKit

@Suite struct FormatTests {
    @Test func latin1RoundTrips() {
        let data = Data((0...255).map(UInt8.init))
        #expect(fromLatin1(latin1(data)) == data)
        #expect(fromLatin1("ā") == nil)
    }

    @Test func bytesAsTextHidesControls() {
        #expect(bytesAsText(Data([0x41, 0x00, 0x0a, 0x7f])) == "A·\n·")
    }

    @Test func numbersLoseTrailingZeros() {
        #expect(numText(612) == "612")
        #expect(numText(0.5) == "0.5")
        #expect(numText(1.256) == "1.26")
        #expect(Rect(x0: 0, y0: 0, x1: 612, y1: 792).sizeText == "612 × 792 pt")
    }

    @Test func pageRangesParse() {
        #expect(parsePages("1-3, 7", count: 10) == [0, 1, 2, 6])
        #expect(parsePages(" 2 ", count: 2) == [1])
        #expect(parsePages("3-2", count: 10) == nil)
        #expect(parsePages("0", count: 10) == nil)
        #expect(parsePages("11", count: 10) == nil)
        #expect(parsePages("a", count: 10) == nil)
        #expect(parsePages("", count: 10) == nil)
    }

    @Test func glyphPathParsesTheBackendsOutlines() throws {
        let p = try #require(glyphPath("M0 0C2 2 4 2 6 0ZM10 -5L20 -5L20 1.5Z"))
        let box = p.boundingBoxOfPath
        #expect(box.minX == 0 && box.maxX == 20)
        #expect(box.minY == -5)
        #expect(glyphPath("M0 0 A1 1") == nil)
        #expect(glyphPath("M0") == nil)
    }
}
