import AppKit
import PdfInspectorKit
import SwiftUI
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
    /// How the next save writes (`nil`: incremental when it can be), as
    /// chosen in the save panel.
    private let saveMode = Mutex<SaveMode?>(nil)
    /// What the last write did, reported once the save is over.
    private let lastSave = Mutex<SaveResult?>(nil)
    private var box: ModelBox?
    var model: DocumentModel? { box?.model }

    private nonisolated var backend: Backend { .shared }

    override nonisolated class var autosavesInPlace: Bool {
        // Saving can mean writing hundreds of MB.
        false
    }

    override nonisolated func read(from url: URL, ofType typeName: String) throws {
        let s: DocSummary
        do {
            s = try open(url, password: nil, recover: false)
        } catch let e as PdfiError where e.code == .parse {
            // The cross-reference data is broken: rebuild it by scanning
            // (the summary says it was recovered).
            s = try open(url, password: nil, recover: true)
        }
        opened.withLock { $0 = s }
    }

    private nonisolated func open(_ url: URL, password: String?, recover: Bool) throws -> DocSummary {
        let reply = try backend.sendNow(
            .open(source: .path(path: url.path), password: password, recover: recover))
        guard case .doc(let s) = reply.body else { throw UnexpectedResponse(body: reply.body) }
        return s
    }

    override func makeWindowControllers() {
        guard let summary = opened.withLock({ $0 }) else { return }
        let box = box ?? ModelBox(DocumentModel(backend: backend, summary: summary))
        self.box = box
        addWindowController(DocumentWindowController(document: self, box: box))
    }

    override func close() {
        if let model {
            model.stop()
            let doc = model.doc
            Task { [backend] in try? await backend.close(doc) }
        }
        super.close()
    }

    /// Open the file again with a password, or rebuilding its
    /// cross-reference data; unsaved edits are dropped.
    func reopen(password: String?, recover: Bool) async throws {
        guard let url = fileURL, let box else { return }
        let s = try open(url, password: password, recover: recover)
        let old = box.model
        old.stop()
        try? await backend.close(old.doc)
        opened.withLock { $0 = s }
        box.model = DocumentModel(backend: backend, summary: s)
        undoManager?.removeAllActions()
        updateChangeCount(.changeCleared)
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

    override func prepareSavePanel(_ savePanel: NSSavePanel) -> Bool {
        guard let model else { return true }
        let options = SaveOptions(status: model.edit)
        saveMode.withLock { $0 = options.mode }
        let view = NSHostingView(rootView: SaveOptionsView(options: options) { [weak self] mode in
            self?.saveMode.withLock { $0 = mode }
        })
        view.frame.size = view.fittingSize
        savePanel.accessoryView = view
        return true
    }

    override nonisolated func write(to url: URL, ofType typeName: String) throws {
        guard let doc = opened.withLock({ $0?.info.doc }) else {
            throw CocoaError(.fileWriteUnknown)
        }
        // Incremental keeps signatures; the backend says when it cannot be.
        let reply = try backend.sendNow(.summary(doc: doc))
        guard case .doc(let s) = reply.body else { throw UnexpectedResponse(body: reply.body) }
        let chosen = saveMode.withLock { $0 }
        let mode: SaveMode =
            if let chosen, chosen != .incremental || s.edit.noIncremental == nil {
                chosen
            } else if s.edit.noIncremental == nil {
                .incremental
            } else {
                .full(xrefStream: true, objectStreams: false, compress: false, removeUnused: false)
            }
        let r = try backend.saveNow(doc, mode: mode, to: url)
        lastSave.withLock { $0 = r }
    }

    override func save(
        to url: URL, ofType typeName: String, for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        super.save(to: url, ofType: typeName, for: saveOperation) { [weak self] error in
            guard let self else { return completionHandler(error) }
            self.saveMode.withLock { $0 = nil }
            if error == nil, let r = self.lastSave.withLock({ $0 }) {
                self.model?.report(
                    r.warnings.isEmpty ? .info : .warning,
                    "Saved \(bytesText(r.bytes)) (\(r.objects.formatted()) objects)", r.warnings)
            }
            if error == nil, saveOperation != .saveToOperation, let model = self.model {
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

/// The choices of the save panel (the web `SaveDialog`).
@MainActor @Observable
final class SaveOptions {
    let status: EditStatus
    var full: Bool
    var xrefStream = true
    var objectStreams = true
    var compress = true
    var removeUnused = false

    init(status: EditStatus) {
        self.status = status
        full = status.noIncremental != nil
    }

    var mode: SaveMode {
        full
            ? .full(
                xrefStream: xrefStream || objectStreams, objectStreams: objectStreams, compress: compress,
                removeUnused: removeUnused)
            : .incremental
    }
}

/// Incremental update or full rewrite, below the save panel.
struct SaveOptionsView: View {
    @Bindable var options: SaveOptions
    let onChange: (SaveMode) -> Void

    var body: some View {
        let s = options.status
        VStack(alignment: .leading, spacing: 6) {
            Text(
                s.dirty
                    ? "\(s.editedObjects.formatted()) object(s) changed since the file was opened."
                    : "No changes: saving writes the file as it is (a full rewrite can still compact it)."
            )
            .foregroundStyle(.secondary)
            Picker("Write", selection: $options.full) {
                VStack(alignment: .leading) {
                    Text("Incremental update")
                    Text(
                        s.noIncremental
                            ?? "The file’s bytes stay as they are; the changed objects and a new cross-reference section are appended. Keeps signatures valid."
                    )
                    .font(.caption).foregroundStyle(.secondary)
                }
                .tag(false)
                .disabled(s.noIncremental != nil)
                VStack(alignment: .leading) {
                    Text("Full rewrite")
                    Text("Every object is written anew: decrypted, without the old revisions.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .tag(true)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            // Always there (the panel keeps the accessory's first size).
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 3) {
                    Toggle("Object streams (smaller; needs PDF 1.5)", isOn: $options.objectStreams)
                    Toggle("Cross-reference stream", isOn: Binding {
                        options.xrefStream || options.objectStreams
                    } set: { options.xrefStream = $0 })
                    .disabled(options.objectStreams)
                    Toggle("Compress streams that have no filter", isOn: $options.compress)
                    Toggle("Leave out objects nothing refers to", isOn: $options.removeUnused)
                }
                .padding(.leading, 20)
                .disabled(!options.full)
            }
            if s.signatures > 0 {
                Text(
                    "\(s.signatures) signed signature field(s). "
                        + (options.full
                            ? "A full rewrite invalidates the signatures."
                            : "An update keeps the signed bytes, but viewers may flag the changes made after signing.")
                )
                .foregroundStyle(options.full ? .red : .orange)
            }
        }
        .font(.callout)
        .padding(12)
        .frame(width: 460, alignment: .leading)
        .onChange(of: options.mode) { onChange(options.mode) }
    }
}

/// Open the file again with a password or by rebuilding its cross-reference
/// data (the web `OpenDialog`'s options).
struct ReopenSheet: View {
    let model: DocumentModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var password = ""
    @State private var recover = false
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Reopen \(model.summary.info.name)").font(.headline)
            if let e = model.summary.encryption {
                Text(
                    "Encrypted (\(e.filter), \(e.streamMethod) \(e.keyBits)-bit)"
                        + (e.authenticated ? "; opened." : ": strings and streams need the password.")
                )
                .font(.caption).foregroundStyle(e.authenticated ? Color.secondary : .red)
            }
            SecureField("Password", text: $password)
            Toggle("Rebuild the cross-reference data by scanning the file", isOn: $recover)
            if model.edit.dirty {
                Text("Unsaved edits are dropped.").font(.caption).foregroundStyle(.orange)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Reopen") { Task { await submit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy)
            }
        }
        .padding()
        .frame(width: 420)
    }

    private func submit() async {
        guard let document else { return }
        busy = true
        defer { busy = false }
        do {
            try await document.reopen(password: password.isEmpty ? nil : password, recover: recover)
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
