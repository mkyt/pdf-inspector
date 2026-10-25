import PdfInspectorKit
import SwiftUI

/// What the value editor does with the value typed.
struct ValueEdit: Identifiable {
    enum Kind {
        /// Replace the node (an empty path: the whole object or the trailer).
        /// `doc`: what the schema says about the entry, to complete its value.
        case set(NodePath, doc: KeyDoc? = nil)
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
    /// Keys of the dictionary an entry is added to, to complete the key and
    /// its value.
    @State private var keys: [TypeKey] = []
    @State private var typeName: String?

    var body: some View {
        let _ = Perf.tick("ValueEditor")
        VStack(alignment: .leading, spacing: 8) {
            Text(edit.title).font(.headline)
            if case .entry = edit.kind {
                TextField("Key", text: $key, prompt: Text("/Name"))
                    .monospaced()
                    .textInputSuggestions {
                        ForEach(keySuggestions, id: \.key) { k in
                            keyRow(k).textInputCompletion("/\(k.key)")
                        }
                    }
                if let typeName {
                    Text("Keys of \(typeName).").font(.caption).foregroundStyle(.secondary)
                }
            }
            Text(description).font(.caption).foregroundStyle(.secondary)
            if loading {
                ProgressView().frame(maxWidth: .infinity, minHeight: 200)
            } else if completes {
                TextField("Value", text: $text)
                    .monospaced()
                    .textInputSuggestions {
                        ForEach(valueSuggestions, id: \.value) { v in
                            valueRow(v).textInputCompletion(v.value)
                        }
                    }
                    .disabled(tooLarge)
                Spacer(minLength: 0)
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
            case .set(let path, _):
                loading = true
                do {
                    let r = try await model.backend.nodeSyntax(model.doc, path: path)
                    text = r.text
                    tooLarge = r.truncated
                } catch {
                    self.error = error.localizedDescription
                }
                loading = false
            case .entry(let path, let k, let v):
                key = k
                text = v
                if let r = try? await model.backend.keys(model.doc, path: path) {
                    typeName = r.typeName
                    keys = r.keys
                }
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

    /// What the schema says about the entry edited.
    private var keyDoc: KeyDoc? {
        switch edit.kind {
        case .set(_, let doc): return doc
        case .entry:
            let k = key.trimmingCharacters(in: .whitespaces)
            let name = k.hasPrefix("/") ? String(k.dropFirst()) : k
            if name.isEmpty { return nil }
            return (keys.first { $0.key == name } ?? keys.first { $0.key == "*" })?.doc
        default: return nil
        }
    }

    /// Values to offer: those the schema lists, else `true` and `false` for
    /// a boolean.
    private var values: [AllowedValue] {
        guard let doc = keyDoc else { return [] }
        if !doc.allowed.isEmpty { return doc.allowed }
        if doc.types.count == 1, doc.types[0].base == "boolean" {
            return ["true", "false"].map { AllowedValue(value: $0, since: nil, deprecated: nil) }
        }
        return []
    }

    /// A value written on several lines is not one of them: edited as text.
    private var completes: Bool {
        !values.isEmpty && !text.trimmingCharacters(in: .whitespacesAndNewlines).contains("\n")
    }

    private var valueSuggestions: [AllowedValue] {
        Self.matching(values, text, \.value)
    }

    /// Keys the dictionary lacks first (required ones before the others),
    /// then those it has.
    private var keySuggestions: [TypeKey] {
        let named = keys.filter { !$0.key.contains("*") }
        let rank = { (k: TypeKey) in (k.present == true ? 2 : 0) + (k.doc.requirement == .required ? 0 : 1) }
        let sorted = named.enumerated()
            .sorted { (rank($0.element), $0.offset) < (rank($1.element), $1.offset) }
            .map(\.element)
        return Self.matching(sorted, key) { "/\($0.key)" }
    }

    /// All of `items` while `search` is empty or one of them; else those
    /// containing it, ignoring case and `/`.
    private static func matching<T>(_ items: [T], _ search: String, _ text: (T) -> String) -> [T] {
        let s = search.trimmingCharacters(in: .whitespaces)
        let q = (s.hasPrefix("/") ? String(s.dropFirst()) : s).lowercased()
        if q.isEmpty || items.contains(where: { PossibleValues.same(text($0), s) }) { return items }
        return items.filter { text($0).lowercased().contains(q) }
    }

    private func keyRow(_ k: TypeKey) -> some View {
        let req: String? = switch k.doc.requirement {
        case .required: "required"
        case .conditional: "conditional"
        case .optional: nil
        }
        let notes = [req, k.doc.since.map { "PDF \($0)" }, k.doc.deprecated.map { "deprecated in \($0)" },
                     k.present == true ? "set" : nil, k.inherited ? "inherited" : nil]
            .compactMap { $0 }.joined(separator: " · ")
        return HStack {
            Text("/\(k.key)").monospaced().strikethrough(k.doc.deprecated != nil)
            Spacer()
            Text(notes).foregroundStyle(.secondary)
        }
    }

    private func valueRow(_ v: AllowedValue) -> some View {
        let since = v.since == keyDoc?.since ? nil : v.since
        let notes = [since.map { "PDF \($0)" }, v.deprecated.map { "deprecated in \($0)" }]
            .compactMap { $0 }.joined(separator: " · ")
        return HStack {
            Text(v.value).monospaced().strikethrough(v.deprecated != nil)
            Spacer()
            Text(notes).foregroundStyle(.secondary)
        }
    }

    private var description: String {
        if completes {
            let open = keyDoc.map { $0.allowedOpen || !$0.constraints.isEmpty } ?? false
            return open ? "Values the schema lists; others are possible too." : "Values the schema lists."
        }
        if case .set(let p, _) = edit.kind, p.path.isEmpty {
            return "Value in PDF syntax. A stream given a dictionary keeps its data."
        }
        return "Value in PDF syntax. References are written 12 0 R; strings (text) or <hex>."
    }

    private var command: EditCommand {
        switch edit.kind {
        case .set(let path, _): .setValue(node: path, value: text)
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
