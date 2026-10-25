import PdfInspectorKit
import SwiftUI

/// What the value editor does with the value typed.
struct ValueEdit: Identifiable {
    enum Kind {
        /// Replace the node (an empty path: the whole object or the trailer).
        case set(NodePath)
        /// Add or replace an entry of the dictionary at the path.
        case entry(NodePath, key: String = "", value: String = "")
        /// Insert into the array at the path.
        case insert(NodePath, index: UInt32)
        /// A new object.
        case new
    }

    let id = UUID()
    let kind: Kind
    let title: String

    init(_ kind: Kind, title: String) {
        self.kind = kind
        self.title = title
    }
}

/// Edit a value written in PDF syntax: `<< /Type /Example >>`,
/// `[0 0 612 792]`, `(text)`, `12 0 R` (the web `ValueEditor`).
struct ValueEditor: View {
    let model: DocumentModel
    let edit: ValueEdit
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var key = ""
    @State private var text = ""
    @State private var loading = false
    @State private var tooLarge = false
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        let _ = Perf.tick("ValueEditor")
        VStack(alignment: .leading, spacing: 8) {
            Text(edit.title).font(.headline)
            if case .entry = edit.kind {
                TextField("Key", text: $key, prompt: Text("/Name"))
                    .monospaced()
            }
            Text(description).font(.caption).foregroundStyle(.secondary)
            if loading {
                ProgressView().frame(maxWidth: .infinity, minHeight: 200)
            } else {
                CodeText(editing: $text)
                    .frame(minHeight: 200, idealHeight: 320)
                    .border(.separator)
                    .disabled(tooLarge)
            }
            if tooLarge {
                Text("Too large to edit as text here; edit its entries one by one.")
                    .font(.caption).foregroundStyle(.orange)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Text("⌘↩ applies. Undo with ⌘Z.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await submit() } }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(busy || tooLarge || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        || (isEntry && key.trimmingCharacters(in: .whitespaces).isEmpty))
            }
        }
        .padding()
        .frame(minWidth: 560, minHeight: 360)
        .task {
            switch edit.kind {
            case .set(let path):
                loading = true
                do {
                    let r = try await model.backend.nodeSyntax(model.doc, path: path)
                    text = r.text
                    tooLarge = r.truncated
                } catch {
                    self.error = error.localizedDescription
                }
                loading = false
            case .entry(_, let k, let v):
                key = k
                text = v
            case .new:
                text = "<<\n  \n>>"
            case .insert:
                break
            }
        }
    }

    private var isEntry: Bool {
        if case .entry = edit.kind { true } else { false }
    }

    private var description: String {
        if case .set(let p) = edit.kind, p.path.isEmpty {
            return "Value in PDF syntax. A stream given a dictionary keeps its data."
        }
        return "Value in PDF syntax. References are written 12 0 R; strings (text) or <hex>."
    }

    private var command: EditCommand {
        switch edit.kind {
        case .set(let path): .setValue(node: path, value: text)
        case .entry(let path, _, _):
            .setEntry(node: path, key: key.hasPrefix("/") ? String(key.dropFirst()) : key, value: text)
        case .insert(let path, let index): .insert(node: path, index: index, value: text)
        case .new: .addObject(value: text, data: nil)
        }
    }

    private func submit() async {
        guard let document else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let r = try await document.perform(edit.title, [command])
            dismiss()
            if case .new = edit.kind, let id = r.created.first { model.nav.open(.object(id)) }
        } catch {
            self.error = error.localizedDescription
        }
    }
}
