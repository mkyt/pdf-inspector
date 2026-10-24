import CoreGraphics
import Foundation
import Testing

@testable import PdfInspectorKit

@Suite struct ContentLayoutTests {
    @Test func foldedBlocksHideTheirRows() {
        // 0 q, 1..3 inside, 4 Q, 5, 6 BT, 7, 8 ET, 9
        let l = OpLayout(folds: [0: 4, 6: 8], total: 10)
        #expect(l.rows == 10 - 4 - 2)
        #expect((0..<l.rows).map(l.op) == [0, 5, 6, 9])
        #expect(l.row(0) == 0)
        #expect(l.row(2) == 0)
        #expect(l.row(5) == 1)
        #expect(l.row(7) == 2)
        #expect(l.row(9) == 3)
    }

    @Test func nestedFoldsCountOnce() {
        let l = OpLayout(folds: [0: 9, 2: 4], total: 10)
        #expect(l.rows == 1)
        #expect(l.op(0) == 0)
    }

    @Test func editsShiftFolds() {
        // Two operations become five before a fold; a fold around the edit grows.
        #expect(remapFolds([10: 12], from: 2, to: 4, count: 5) == [13: 15])
        #expect(remapFolds([0: 20], from: 2, to: 4, count: 5) == [0: 23])
        #expect(remapFolds([3: 6], from: 2, to: 4, count: 5) == [:])
        #expect(remapFolds([0: 1], from: 2, to: 4, count: 5) == [0: 1])
    }

    @Test func movesKeepWholeBlocks() {
        // Move 0..<2 before 5: the block 2...4 moves up by two.
        #expect(moveFolds([2: 4], from: 0, to: 2, before: 5) == [0: 2])
        // A block split by the move unfolds.
        #expect(moveFolds([1: 3], from: 0, to: 2, before: 5) == [:])
    }

    @Test func viewTransformRoundTrips() {
        // A page 612 × 792 seen from the top left.
        let v = ViewInfo(width: 612, height: 792, matrix: [1, 0, 0, -1, 0, 792], rotate: 0)
        let r = v.toView(Rect(x0: 72, y0: 700, x1: 144, y1: 720))
        #expect(r == CGRect(x: 72, y: 72, width: 72, height: 20))
        let back = v.toContent(r)
        #expect(back == Rect(x0: 72, y0: 700, x1: 144, y1: 720))
    }
}
