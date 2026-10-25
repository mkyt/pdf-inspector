import Observation
import Synchronization
import Testing

@testable import PdfInspectorKit

@MainActor
@Suite struct NavigationTests {
    @Test func theSelectedOperationIsApartFromThePlace() {
        let nav = Navigation()
        let c = ContentRef.page(index: 2)
        nav.open(.content(c, op: 5))
        #expect(nav.target == .content(c, op: 5))
        #expect(nav.place == .content(c, op: nil))
        #expect(nav.op == 5)
        nav.update(.content(c, op: 9))
        #expect(nav.target == .content(c, op: 9))
        nav.open(.page(2))
        #expect(nav.op == nil)
        nav.goBack()
        #expect(nav.target == .content(c, op: 9))
    }

    @Test func selectingAnOperationLeavesReadersOfThePlaceAlone() {
        let nav = Navigation()
        let c = ContentRef.page(index: 0)
        nav.open(.content(c, op: 0))
        let placeChanged = Flag()
        withObservationTracking { _ = nav.place } onChange: { placeChanged.set() }
        let opChanged = Flag()
        withObservationTracking { _ = nav.op } onChange: { opChanged.set() }
        nav.update(.content(c, op: 1))
        #expect(!placeChanged.value)
        #expect(opChanged.value)
    }

}

private final class Flag: Sendable {
    private let state = Mutex(false)
    var value: Bool { state.withLock { $0 } }
    func set() { state.withLock { $0 = true } }
}
