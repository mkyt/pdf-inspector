import PdfInspectorKit
import SwiftUI

/// The right-hand panel (the web `SchemaPanel`): what the schema says about
/// the selected entry (or, with none selected, about the object shown), or
/// what the selected operation of a content does.
struct SchemaPane: View {
    let model: DocumentModel

    var body: some View {
        let _ = Perf.tick("SchemaPane")
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                content
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder private var content: some View {
        let nav = model.nav
        switch nav.place {
        case .content(let c, _):
            if let op = nav.op {
                OperatorDoc(model: model, content: c, op: op)
                    .id(op)
            } else if case .stream(let id) = c {
                TypeView(model: model, root: .object(id: id), title: id.objText)
            } else if case .page(let index) = c {
                PageType(model: model, index: index)
            }
        default:
            if let s = nav.selection {
                SelectionView(model: model, selection: s)
            } else {
                switch nav.place {
                case .object(let id):
                    TypeView(model: model, root: .object(id: id), title: id.objText, structure: true)
                case .trailer:
                    TypeView(model: model, root: .trailer, title: "Trailer", structure: true)
                case .document:
                    if let root = model.summary.root {
                        TypeView(model: model, root: .object(id: root), title: root.objText)
                    } else {
                        Text("The trailer has no /Root: the document has no catalog to describe.")
                            .font(.caption).foregroundStyle(.red)
                    }
                case .page(let index):
                    PageType(model: model, index: index)
                case .font(let id):
                    TypeView(model: model, root: .object(id: id), title: id.objText)
                case .content:
                    EmptyView()
                }
            }
        }
    }
}

private struct SelectionView: View {
    let model: DocumentModel
    let selection: Selection

    var body: some View {
        let _ = Perf.tick("SelectionView")
        let s = selection.node.schema
        VStack(alignment: .leading, spacing: 2) {
            Text(selection.path).font(.headline.monospaced()).textSelection(.enabled)
            Text(selection.node.value.text).font(.caption.monospaced()).foregroundStyle(.secondary)
                .textSelection(.enabled)
        }
        if let t = s?.valueType {
            HStack {
                Text("Value type").font(.caption).foregroundStyle(.secondary)
                Badge(t, color: .accentColor)
            }
        }
        if let k = s?.key, k.hasValues {
            SectionLabel("Possible values")
            PossibleValues(doc: k, current: selection.node.value.text)
        }
        if let s, !s.issues.isEmpty {
            SectionLabel("Issues")
            IssueList(model: model, issues: s.issues, max: 20)
        }
        SectionLabel("Definition")
        if let s {
            if let k = s.key {
                KeyDocView(doc: k, values: false)
            } else {
                Text("Not defined by the schema for this type.").font(.caption).foregroundStyle(.secondary)
            }
        } else {
            Text("The type of the containing object is not known.").font(.caption).foregroundStyle(.secondary)
        }
    }
}

struct SectionLabel: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        let _ = Perf.tick("SectionLabel")
        HStack(spacing: 6) {
            Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            VStack { Divider() }
        }
        .padding(.top, 4)
    }
}

private struct PageType: View {
    let model: DocumentModel
    let index: UInt32

    var body: some View {
        let _ = Perf.tick("PageType")
        Loaded(model.generation, fetch: { try await model.backend.page(model.doc, index: index) }) { p in
            TypeView(model: model, root: .object(id: p.id), title: p.id.objText)
        }
    }
}

/// The schema type of an object (or the trailer) and the keys it defines.
/// `structure`: its structure is shown beside, to select entries from;
/// otherwise the title opens the object.
struct TypeView: View {
    let model: DocumentModel
    let root: NodeRoot
    let title: String
    var structure = false
    @State private var open: String?
    @State private var extensions = false

    private struct Key: Equatable {
        let root: NodeRoot
        let generation: Int
    }

    var body: some View {
        let _ = Perf.tick("TypeView")
        Loaded(Key(root: root, generation: model.generation), fetch: {
            try await model.backend.schema(model.doc, root: root)
        }) { s in
            view(s)
        }
    }

    private func originText(_ o: TypeOrigin?) -> String {
        switch o {
        case .guess: "guessed from /Type"
        case .typeKey: "named by its /Type"
        default: "typed by the references reaching it"
        }
    }

    @ViewBuilder private func view(_ s: ObjectSchema) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                if structure || root == .trailer {
                    Text(title).font(.headline.monospaced())
                } else {
                    LinkButton(title) { model.nav.open(root.target) }.font(.headline.monospaced())
                }
                if let t = s.typeName {
                    HStack(spacing: 6) {
                        Badge(t + (s.origin == .guess ? "?" : ""), color: .teal, outline: s.origin == .guess)
                        Text(originText(s.origin))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if s.typeName == nil {
                Text(
                    "The schema type of this object is not known. Open it through a reference, or validate the document, to type it from where it is used."
                )
                .font(.caption).foregroundStyle(.secondary)
            } else {
                if s.note != nil || s.pdfVersion != nil {
                    Text(
                        (s.note.map { "Defined in \($0). " } ?? "")
                            + (s.pdfVersion.map { "Checked against PDF \($0)." } ?? "")
                    )
                    .font(.caption).foregroundStyle(.secondary)
                }
                let standard = s.keys.filter { $0.doc.extension == nil }
                let extra = s.keys.count - standard.count
                let shown = extensions ? s.keys : standard
                if !shown.isEmpty {
                    SectionLabel(
                        "Keys: \(standard.filter { $0.present == true || $0.inherited }.count) of \(standard.count) present")
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(shown, id: \.key) { k in
                            TypeKeyRow(k: k, open: open == k.key) { open = open == k.key ? nil : k.key }
                            if open == k.key {
                                KeyDocView(doc: k.doc).padding(.leading, 14).padding(.bottom, 6)
                            }
                        }
                    }
                    if extra > 0 {
                        LinkButton("\(extensions ? "Hide" : "Show") \(extra) key\(extra == 1 ? "" : "s") from extensions") {
                            extensions.toggle()
                        }
                        .font(.caption)
                    }
                }
            }
            if structure {
                Text("Select an entry in the structure for the definition of its value.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

private struct TypeKeyRow: View {
    let k: TypeKey
    let open: Bool
    let toggle: () -> Void

    var body: some View {
        let _ = Perf.tick("TypeKeyRow")
        let old = k.doc.deprecated != nil
        HStack(spacing: 4) {
            Button(action: toggle) {
                HStack(spacing: 2) {
                    Image(systemName: open ? "chevron.down" : "chevron.right").font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(keyLabel)
                        .monospaced()
                        .strikethrough(old)
                        .foregroundStyle(k.doc.extension != nil || old ? .secondary : Color.indigo)
                }
            }
            .buttonStyle(.plain)
            switch k.doc.requirement {
            case .required: Text("required").foregroundStyle(.red)
            case .conditional(let c): Text("conditional").foregroundStyle(.orange).help(c)
            case .optional: EmptyView()
            }
            Spacer()
            if k.present == true {
                Text("present").foregroundStyle(.teal)
            } else if k.inherited {
                Text("inherited").foregroundStyle(.teal)
                    .help("Not in this dictionary: given by an ancestor through /Parent")
            } else if k.present == false {
                if k.doc.requirement == .required {
                    Text("missing").foregroundStyle(.red)
                } else {
                    Text("—").foregroundStyle(.secondary)
                }
            }
        }
        .font(.caption)
    }

    private var keyLabel: String {
        if k.key == "*" { return "* (any)" }
        if k.key.first?.isNumber == true { return "[\(k.key)]" }
        return "/\(k.key)"
    }
}

struct KeyDocView: View {
    let doc: KeyDoc
    /// Include the possible values (shown apart for a selected entry).
    var values = true

    var body: some View {
        let _ = Perf.tick("KeyDocView")
        Grid(alignment: .topLeading, horizontalSpacing: 8, verticalSpacing: 3) {
            GridRow {
                label("Type")
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(doc.types.enumerated()), id: \.offset) { _, t in
                        Text("\(Text(t.base).bold())\(Text(t.links.isEmpty ? "" : " → " + t.links.joined(separator: ", ")).foregroundStyle(.secondary))")
                    }
                }
            }
            GridRow {
                label("Required")
                switch doc.requirement {
                case .conditional(let c): Text("when \(Text(c).monospaced())")
                case .required: Badge("required", color: .red)
                case .optional: Badge("optional")
                }
            }
            if let since = doc.since {
                row("Since", "PDF \(since)" + (doc.extension.map { " (extension \($0))" } ?? ""))
            } else if let e = doc.extension {
                row("Extension", e)
            }
            if let d = doc.deprecated { row("Deprecated", "PDF \(d)") }
            if doc.inheritable { row("Inheritable", "yes") }
            if let d = doc.default {
                GridRow {
                    label("Default")
                    Text(d).monospaced()
                }
            }
            if values && doc.hasValues {
                GridRow {
                    label("Values")
                    PossibleValues(doc: doc)
                }
            }
            if let n = doc.note { row("Note", n) }
        }
        .font(.caption)
        .textSelection(.enabled)
    }

    private func label(_ t: String) -> some View {
        Text(t).foregroundStyle(.secondary).frame(width: 72, alignment: .leading)
    }

    private func row(_ l: String, _ v: String) -> some View {
        GridRow {
            label(l)
            Text(v)
        }
    }
}

extension KeyDoc {
    var hasValues: Bool { !allowed.isEmpty || !constraints.isEmpty }
}

/// What the value of a key may be: the values the schema lists (`current`
/// marked, deprecated ones struck through), the conditions it meets, and
/// whether there are others.
struct PossibleValues: View {
    let doc: KeyDoc
    var current: String?

    var body: some View {
        let _ = Perf.tick("PossibleValues")
        VStack(alignment: .leading, spacing: 4) {
            if !doc.allowed.isEmpty {
                FlowLayout(spacing: 3) {
                    ForEach(Array(doc.allowed.enumerated()), id: \.offset) { _, v in
                        chip(v)
                    }
                }
            }
            ForEach(doc.constraints, id: \.self) { c in
                Text("where \(Text(c).monospaced())")
            }
            if doc.allowedOpen {
                Text("Others are possible too.").foregroundStyle(.secondary)
            }
        }
        .font(.caption)
    }

    private func chip(_ v: AllowedValue) -> some View {
        let on = current.map { same(v.value, $0) } ?? false
        let since = v.since == doc.since ? nil : v.since
        return HStack(spacing: 2) {
            if on { Image(systemName: "checkmark").font(.caption2.bold()) }
            Text(v.value).monospaced().strikethrough(v.deprecated != nil)
            if let since { Text(since).font(.caption2).foregroundStyle(.secondary) }
        }
        .padding(.horizontal, 5)
        .padding(.vertical, 1)
        .foregroundStyle(on ? Color.accentColor : v.deprecated != nil ? .secondary : .primary)
        .background(on ? Color.accentColor.opacity(0.15) : Color.secondary.opacity(0.1), in: .rect(cornerRadius: 4))
        .help(
            [on ? "The current value." : nil, v.since.map { "Since PDF \($0)." },
             v.deprecated.map { "Deprecated in PDF \($0)." }]
                .compactMap { $0 }.joined(separator: " "))
    }

    /// Same value, numbers by value (`1` and `1.0`).
    private func same(_ a: String, _ b: String) -> Bool {
        if a == b { return true }
        guard let x = Double(a), let y = Double(b) else { return false }
        return x == y
    }
}
