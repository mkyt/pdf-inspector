import AppKit
import PdfInspectorKit
import Synchronization

/// One PDF, opened in the shared Rust session by path (docs/DESIGN.md §4.10):
/// the backend reads the file where it is, however large.
///
/// Edits live in the backend's history; each one is mirrored in the undo
/// manager, which keeps NSDocument's edited state. Saving writes the
/// document with its edits to the location NSDocument hands out, which then
/// replaces the original; the backend reopens it from there.
final class InspectorDocument: NSDocument {
    /// Set by `read` (which NSDocument may call off the main thread).
    private let opened = Mutex<DocSummary?>(nil)
    private(set) var model: DocumentModel?

    private nonisolated var backend: Backend { .shared }

    override nonisolated class var autosavesInPlace: Bool {
        // Saving can mean writing hundreds of MB.
        false
    }

    override nonisolated func read(from url: URL, ofType typeName: String) throws {
        let reply = try backend.sendNow(
            .open(source: .path(path: url.path), password: nil, recover: false))
        guard case .doc(let s) = reply.body else { throw UnexpectedResponse(body: reply.body) }
        opened.withLock { $0 = s }
    }

    override func makeWindowControllers() {
        guard let summary = opened.withLock({ $0 }) else { return }
        let model = model ?? DocumentModel(backend: backend, summary: summary)
        self.model = model
        addWindowController(DocumentWindowController(document: self, model: model))
    }

    override func close() {
        if let model {
            model.stop()
            let doc = model.doc
            Task { [backend] in try? await backend.close(doc) }
        }
        super.close()
    }

    // MARK: Editing

    /// Apply commands as one undoable step.
    @discardableResult
    func perform(_ label: String, _ commands: [EditCommand]) async throws -> ApplyResult {
        guard let model else { throw CocoaError(.featureUnsupported) }
        let r = try await model.apply(label, commands)
        registerStep(label, undo: true)
        return r
    }

    /// Mirror a backend step in the undo manager. The inverse is registered
    /// at once (inside the undo or redo), the backend follows.
    private func registerStep(_ label: String, undo: Bool) {
        guard let undoManager else { return }
        undoManager.registerUndo(withTarget: self) { doc in
            doc.registerStep(label, undo: !undo)
            Task { @MainActor in
                do {
                    if undo { try await doc.model?.undo() } else { try await doc.model?.redo() }
                } catch {
                    doc.presentError(error)
                }
            }
        }
        undoManager.setActionName(label)
    }

    // MARK: Saving

    override nonisolated func write(to url: URL, ofType typeName: String) throws {
        guard let doc = opened.withLock({ $0?.info.doc }) else {
            throw CocoaError(.fileWriteUnknown)
        }
        // Incremental keeps signatures; the backend says when it cannot be.
        // TODO: offer full rewrites (xref stream, object streams, compress,
        // drop unused) in the save panel, like the web SaveDialog.
        let reply = try backend.sendNow(.summary(doc: doc))
        guard case .doc(let s) = reply.body else { throw UnexpectedResponse(body: reply.body) }
        let mode: SaveMode =
            s.edit.noIncremental == nil
            ? .incremental
            : .full(xrefStream: true, objectStreams: false, compress: false, removeUnused: false)
        _ = try backend.saveNow(doc, mode: mode, to: url)
    }

    override func save(
        to url: URL, ofType typeName: String, for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        super.save(to: url, ofType: typeName, for: saveOperation) { [weak self] error in
            if error == nil, saveOperation != .saveToOperation, let self, let model = self.model {
                // The file is in place: read the document from it, with a
                // fresh history.
                do {
                    model.reloaded(try self.backend.reload(model.doc, from: url))
                    self.undoManager?.removeAllActions()
                } catch {
                    completionHandler(error)
                    return
                }
            }
            completionHandler(error)
        }
    }

    override func revert(toContentsOf url: URL, ofType typeName: String) throws {
        guard let model else { return try super.revert(toContentsOf: url, ofType: typeName) }
        model.reloaded(try backend.reload(model.doc, from: url))
        undoManager?.removeAllActions()
        updateChangeCount(.changeCleared)
    }
}
