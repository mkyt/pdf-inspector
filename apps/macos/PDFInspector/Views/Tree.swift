import PdfInspectorKit
import SwiftUI

/// Opens the value editor.
struct EditValueAction {
    let open: (ValueEdit) -> Void

    func callAsFunction(_ e: ValueEdit) { open(e) }
}

extension EnvironmentValues {
    /// Opens the value editor; without it the tree is read-only.
    @Entry var editValue: EditValueAction?
}

/// Children of `path`, fetched page by page as asked (the web
/// `NodeChildren`). `label` is where `path` is, for the schema panel
/// (`/Pages/Kids[3]`).
struct NodeChildren: View {
    let model: DocumentModel
    let path: NodePath
    let depth: Int
    var label = ""
    @State private var nodes: [NodeView] = []
    @State private var total: UInt64?
    @State private var error: String?
    @State private var loading = false

    private static let page: UInt32 = 200
    private var indent: CGFloat { CGFloat(depth) * 16 + 20 }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(nodes, id: \.index) { n in
                NodeRow(model: model, parent: path, node: n, depth: depth, label: label)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).padding(.leading, indent)
            } else if total == nil {
                ProgressView().controlSize(.mini).padding(.leading, indent)
            } else if let total, UInt64(nodes.count) < total {
                Button("Show more (\(nodes.count.formatted()) of \(total.formatted()))") {
                    Task { await more() }
                }
                .buttonStyle(.link)
                .font(.caption)
                .disabled(loading)
                .padding(.leading, indent)
                .padding(.vertical, 2)
            }
        }
        .task(id: model.generation) {
            nodes = []
            total = nil
            await more()
        }
    }

    private func more() async {
        loading = true
        defer { loading = false }
        do {
            let p = try await model.backend.children(
                model.doc, path: path, range: ListRange(offset: UInt64(nodes.count), limit: Self.page))
            nodes += p.items
            total = p.total
            error = nil
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private let maxDepth = 32

struct NodeRow: View {
    let model: DocumentModel
    let parent: NodePath
    let node: NodeView
    let depth: Int
    let label: String
    @State private var open = false
    @Environment(\.editValue) private var editValue
    @Environment(\.inspectorDocument) private var document

    private var keyText: String { node.key.map { "/\($0)" } ?? "[\(node.index)]" }
    private var here: String { label + keyText }

    private var childPath: NodePath {
        if case .ref(let id) = node.value { return NodePath(root: .object(id: id), path: []) }
        return NodePath(root: parent.root, path: parent.path + [node.index])
    }

    var body: some View {
        let v = node.value
        let expandable: Bool = {
            if case .ref = v { return depth < maxDepth }
            return depth < maxDepth && v.hasChildren
        }()
        let selected = model.nav.selection?.path == here && model.nav.selection?.node.index == node.index
        let schema = node.schema
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 4) {
                Button {
                    open.toggle()
                } label: {
                    Image(systemName: open ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                        .frame(width: 12)
                        .opacity(expandable ? 1 : 0)
                }
                .buttonStyle(.plain)
                .disabled(!expandable)
                Text(keyText)
                    .monospaced()
                    .foregroundStyle(schema != nil && schema?.key == nil ? .orange : .primary)
                    .help(schema != nil && schema?.key == nil ? "Not defined by the schema for this type" : "")
                if case .ref(let id) = v {
                    LinkButton(id.refText) { model.nav.open(.object(id)) }
                        .monospaced()
                } else {
                    Text(v.text).monospaced().lineLimit(1).foregroundStyle(v.color)
                }
                if let t = schema?.valueType {
                    Text(t).foregroundStyle(.teal)
                }
                Spacer(minLength: 4)
                if let schema, let w = schema.issues.worst {
                    Circle()
                        .fill(w.color)
                        .frame(width: 7, height: 7)
                        .help(schema.issues.map { ($0.path.map { "\($0): " } ?? "") + $0.message }
                            .joined(separator: "\n"))
                }
            }
            .font(.callout)
            .padding(.leading, CGFloat(depth) * 16 + 4)
            .padding(.vertical, 1)
            .padding(.trailing, 6)
            .background(selected ? Color.accentColor.opacity(0.2) : .clear)
            .contentShape(.rect)
            .onTapGesture {
                // Clicking the selected entry again deselects it.
                if selected { return model.nav.select(nil) }
                model.nav.select(Selection(node: node, path: here))
                if let s = node.span { model.nav.showBytes(s) }
            }
            .help(node.span.map { "bytes \($0.text)" } ?? "")
            .contextMenu { menu }
            if open && expandable {
                // Following a reference starts a new location: the referenced object.
                let childLabel = if case .ref = v { "" } else { here }
                NodeChildren(model: model, path: childPath, depth: depth + 1, label: childLabel)
            }
        }
    }

    /// Edits of one entry: its value, what it holds, removing it.
    @ViewBuilder private var menu: some View {
        if let editValue {
            let v = node.value
            let path = NodePath(root: parent.root, path: parent.path + [node.index])
            let name = here.isEmpty ? "/" : here
            Button("Edit Value…") { editValue(ValueEdit(.set(path), title: "Edit \(name)")) }
            switch v {
            case .dict, .stream:
                Button("Add Entry…") { editValue(ValueEdit(.entry(path), title: "Add an entry to \(name)")) }
            case .array(let len):
                Button("Append Item…") {
                    editValue(ValueEdit(.insert(path, index: UInt32(len)), title: "Append to \(name)"))
                }
            default:
                EmptyView()
            }
            if node.key == nil {
                Button("Insert Item Before…") {
                    editValue(ValueEdit(.insert(parent, index: node.index), title: "Insert before \(name)"))
                }
            }
            Divider()
            Button("Remove", role: .destructive) {
                document?.run("Remove \(name)", [.removeChild(node: parent, index: node.index)])
            }
        }
        if case .ref(let id) = node.value {
            Divider()
            Button("Open \(id.objText)") { model.nav.open(.object(id)) }
        }
        if let s = node.span {
            Button("Show in File Bytes") { model.nav.showBytes(s, reveal: true) }
        }
    }
}

extension Value {
    var color: Color {
        switch self {
        case .string: .brown
        case .name: .purple
        case .integer, .real: .blue
        case .bool, .null: .pink
        default: .secondary
        }
    }
}
