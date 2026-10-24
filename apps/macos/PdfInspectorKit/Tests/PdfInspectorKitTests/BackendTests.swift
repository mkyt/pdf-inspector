import CoreGraphics
import Foundation
import Testing

@testable import PdfInspectorKit

/// `tests/fixtures` of the repository.
private func fixture(_ name: String) -> URL {
    URL(filePath: #filePath)
        .deletingLastPathComponent()
        .appending(path: "../../../../../tests/fixtures/\(name)")
        .standardized
}

@Suite struct BackendTests {
    let backend = Backend()

    @Test func helloMatchesTheBindings() async throws {
        let h = try await backend.hello()
        #expect(h.protocol == protocolVersion())
        #expect(h.backend == "native")
    }

    @Test func errorsCarryTheirCode() async throws {
        await #expect {
            try await backend.summary(999)
        } throws: { e in
            (e as? PdfiError)?.code == .noSuchDoc
        }
        await #expect {
            try await backend.open(path: "/nonexistent.pdf")
        } throws: { e in
            (e as? PdfiError)?.code == .notFound
        }
    }

    @Test func openAndReadByRange() async throws {
        let s = try await backend.open(path: fixture("base.pdf").path)
        #expect(s.info.name == "base.pdf")
        #expect(s.pages > 0)
        let objects = try await backend.objects(s.info.doc, range: ListRange(offset: 0, limit: 3))
        #expect(objects.items.count == 3)
        #expect(objects.total == s.xrefEntries)
        let pages = try await backend.pages(s.info.doc, range: ListRange(offset: 0, limit: 100))
        #expect(pages.items.count == Int(s.pages))
        let root = try #require(s.root)
        let catalog = try await backend.object(s.info.doc, id: root)
        #expect(catalog.id == root)
    }

    @Test func renderGivesAnImage() async throws {
        let s = try await backend.open(path: fixture("render.pdf").path)
        let b = try await backend.render(s.info.doc, .page(index: 0), scale: 0.5)
        #expect(b.width > 0 && b.height > 0)
        #expect(b.data.count == b.width * b.height * 4)
        let image = try #require(b.cgImage())
        #expect(image.width == b.width)
    }

    @Test func cancellingTheTaskStopsTheRequest() async throws {
        let s = try await backend.open(path: fixture("render.pdf").path)
        let t = Task { try await backend.render(s.info.doc, .page(index: 0), scale: 8) }
        t.cancel()
        await #expect(throws: (any Error).self) { try await t.value }
    }

    @Test func jobsReportThroughEvents() async throws {
        let s = try await backend.open(path: fixture("model.pdf").path)
        let events = backend.events()
        let j = try await backend.startJob(s.info.doc, .inventory)
        for await ev in events {
            if case .jobDone(let info) = ev, info.job == j.job {
                #expect(info.state == .completed)
                break
            }
        }
        let fonts = try await backend.fonts(s.info.doc, range: ListRange(offset: 0, limit: 10))
        #expect(fonts.total > 0)
    }

    @Test func editUndoSaveAndReload() async throws {
        let s = try await backend.open(path: fixture("base.pdf").path)
        let doc = s.info.doc
        let r = try await backend.apply(
            doc, label: "Set title",
            commands: [.setInfo(key: "Title", value: "Edited", syncXmp: false)])
        #expect(r.status.dirty)
        #expect(r.status.undo == "Set title")
        let undone = try await backend.undo(doc)
        #expect(!undone.dirty)
        _ = try await backend.redo(doc)

        let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appending(path: "saved.pdf")
        let saved = try await backend.save(doc, mode: .incremental, to: url)
        #expect(saved.bytes > 0)
        // Never over an existing file.
        await #expect(throws: PdfiError.self) {
            try await backend.save(doc, mode: .incremental, to: url)
        }
        let again = try backend.reload(doc, from: url)
        #expect(again.info.path == url.path)
        #expect(!again.edit.dirty)
        let view = try await backend.document(doc)
        #expect(view.infoEntries.contains { $0.key == "Title" && $0.text == "Edited" })
    }

    @MainActor @Test func pagedListFetchesWhatIsNeeded() async throws {
        let s = try await backend.open(path: fixture("base.pdf").path)
        let list = PagedList<ObjectSummaryPage>(total: Int(s.xrefEntries)) { [backend] r in
            try await backend.objects(s.info.doc, range: r)
        }
        #expect(list.item(0) == nil)
        list.need(0)
        while list.item(0) == nil && list.error == nil {
            await Task.yield()
        }
        #expect(list.item(0)?.num == 0)
        #expect(list.total == Int(s.xrefEntries))
    }

    @MainActor @Test func navigationKeepsHistory() {
        let nav = Navigation()
        nav.open(.trailer)
        nav.open(.page(2))
        nav.update(.page(3))
        #expect(nav.canGoBack)
        nav.goBack()
        #expect(nav.target == .trailer)
        nav.goForward()
        #expect(nav.target == .page(3))
        #expect(Target.content(.page(index: 0), op: 1).isSame(as: .content(.page(index: 0), op: nil)))
    }
}
