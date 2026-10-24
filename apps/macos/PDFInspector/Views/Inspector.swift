import PdfInspectorKit
import SwiftUI

/// The trailer or an object: structure, schema type, stream data, source
/// (the web `Inspector`).
struct InspectorView: View {
    let model: DocumentModel
    let root: NodeRoot
    @State private var edit: ValueEdit?

    var body: some View {
        Group {
            switch root {
            case .trailer:
                TrailerInspector(model: model)
            case .object(let id):
                ObjectInspector(model: model, id: id)
                    .id(id)
            }
        }
        .environment(\.editValue, EditValueAction { edit = $0 })
        .sheet(item: $edit) { e in
            ValueEditor(model: model, edit: e)
        }
    }
}

private struct TrailerInspector: View {
    let model: DocumentModel
    @State private var schema: ObjectSchema?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("Trailer").font(.title3.bold())
                    SchemaBadge(schema: schema)
                    EditMenu(model: model, root: .trailer, isDict: true, name: "the trailer")
                }
                Text("Merged trailer of all revisions.").font(.caption).foregroundStyle(.secondary)
                SchemaIssues(model: model, schema: schema)
            }
            .padding(10)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    NodeChildren(model: model, path: NodePath(root: .trailer, path: []), depth: 0)
                }
                .padding(.vertical, 4)
            }
        }
        .task(id: model.generation) {
            schema = try? await model.backend.schema(model.doc, root: .trailer)
        }
    }
}

private struct ObjectInspector: View {
    let model: DocumentModel
    let id: ObjRef
    @State private var object: ObjectView?
    @State private var schema: ObjectSchema?
    @State private var error: String?
    @State private var tab: Tab = .structure

    enum Tab: Hashable { case structure, stream, source }

    var body: some View {
        Group {
            if let o = object {
                content(o)
            } else if let error {
                ContentUnavailableView(id.objText, systemImage: "exclamationmark.triangle", description: Text(error))
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: model.generation) {
            do {
                object = try await model.backend.object(model.doc, id: id)
                error = nil
            } catch is CancellationError {
            } catch {
                self.error = error.localizedDescription
                object = nil
            }
            schema = try? await model.backend.schema(model.doc, root: .object(id: id))
        }
    }

    private func content(_ o: ObjectView) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            header(o)
                .padding(10)
            Picker("View", selection: $tab) {
                Text("Structure").tag(Tab.structure)
                if o.stream != nil { Text("Stream data").tag(Tab.stream) }
                Text("Source").tag(Tab.source)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            .padding(.horizontal, 10)
            .padding(.bottom, 6)
            Divider()
            switch tab {
            case .structure:
                structure(o)
            case .stream:
                if let info = o.stream {
                    StreamPanel(model: model, id: o.id, info: info)
                }
            case .source:
                SourceView(model: model, object: o)
            }
        }
    }

    @ViewBuilder private func header(_ o: ObjectView) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(o.id.objText).font(.title3.bold()).textSelection(.enabled)
                SchemaBadge(schema: schema)
                Badge(o.value.text)
                switch o.location {
                case .edited:
                    Badge("edited, not saved", color: .orange)
                case .file(let span):
                    LinkButton("file \(span.text)") { model.nav.showBytes(span, reveal: true) }
                        .font(.caption)
                case .objStm(let stream, let index):
                    LinkButton("in object stream \(stream.num) \(stream.generation) [index \(index)]") {
                        model.nav.open(.object(stream))
                    }
                    .font(.caption)
                }
                Spacer()
                EditMenu(model: model, root: .object(id: o.id), isDict: o.value.isDictLike, name: o.id.objText)
            }
            ForEach(Array(o.diagnostics.enumerated()), id: \.offset) { _, d in
                DiagnosticRow(d: d) { model.nav.open(.object($0)) }
            }
            SchemaIssues(model: model, schema: schema)
            links(o)
            Referrers(model: model, id: o.id)
            if let info = o.stream {
                ImagePreview(model: model, id: o.id, info: info)
            }
        }
    }

    @ViewBuilder private func links(_ o: ObjectView) -> some View {
        if case .dict(_, "Page") = o.value {
            PageLink(model: model, id: o.id)
        }
        if case .dict = o.value, let t = schema?.typeName, t.wholeMatch(of: fontType) != nil {
            LinkButton("Program, encoding and glyphs →") { model.nav.open(.font(o.id)) }.font(.caption)
        }
        if o.stream != nil, let s = schema, s.typeName.map({ $0.firstMatch(of: notContent) == nil }) ?? true {
            LinkButton("Read as a content stream →") {
                model.nav.open(.content(.stream(id: o.id), op: nil))
            }
            .font(.caption)
        }
    }

    @ViewBuilder private func structure(_ o: ObjectView) -> some View {
        switch o.value {
        case .dict, .array, .stream:
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    NodeChildren(model: model, path: NodePath(root: .object(id: o.id), path: []), depth: 0)
                }
                .padding(.vertical, 4)
            }
        default:
            Text(o.value.text).monospaced().textSelection(.enabled).padding(10)
            Spacer()
        }
    }
}

/// Font dictionaries (by schema type; descendant CID fonts are reached from
/// their Type 0 font).
private let fontType = /Font(Type[013]|TrueType|MultipleMaster)/

/// Streams that are not content of any kind (by schema type).
private let notContent =
    /Image|XRef|ObjStm|ObjectStream|Metadata|FontFile|ICC|EmbeddedFile|CMap|Function|Shading|Thumbnail|Halftone|JavaScript|SoftMask$/

extension Value {
    var isDictLike: Bool {
        switch self {
        case .dict, .stream: true
        default: false
        }
    }

    var typeName: String? {
        switch self {
        case .dict(_, let t), .stream(_, let t, _): t
        default: nil
        }
    }
}

/// Edits of a whole object or the trailer.
private struct EditMenu: View {
    let model: DocumentModel
    let root: NodeRoot
    let isDict: Bool
    let name: String
    @Environment(\.editValue) private var editValue
    @Environment(\.inspectorDocument) private var document

    var body: some View {
        let path = NodePath(root: root, path: [])
        Menu("Edit") {
            Button("Edit as Text…") { editValue?(ValueEdit(.set(path), title: "Edit \(name)")) }
            if isDict {
                Button("Add Entry…") { editValue?(ValueEdit(.entry(path), title: "Add an entry to \(name)")) }
            }
            Button("New Object…") { editValue?(ValueEdit(.new, title: "New object")) }
            if case .object(let id) = root {
                Divider()
                Button("Delete Object", role: .destructive) {
                    document?.run("Delete \(name)", [.deleteObject(id: id)])
                    model.nav.goBack()
                }
            }
        }
        .menuStyle(.button)
        .controlSize(.small)
        .fixedSize()
    }
}

/// How the schema type of an object was found, for its tooltip.
extension TypeOrigin {
    var tip: String {
        switch self {
        case .context: "Typed by following references from the trailer."
        case .typeKey: "Named by its /Type, which only this type takes; not checked against where it is used."
        case .guess:
            "Guessed from /Type among several types taking it; open it through a reference or validate the document to type it from context."
        }
    }
}

/// Schema type of the object; a guess (from `/Type`) is marked as such.
struct SchemaBadge: View {
    let schema: ObjectSchema?

    var body: some View {
        if let s = schema, let t = s.typeName {
            let guess = s.origin == .guess
            Badge(t + (guess ? "?" : ""), color: .teal, outline: guess)
                .help([s.origin?.tip, s.note, s.pdfVersion.map { "Checked against PDF \($0)." }]
                    .compactMap { $0 }.joined(separator: " "))
        }
    }
}

struct SchemaIssues: View {
    let model: DocumentModel
    let schema: ObjectSchema?

    var body: some View {
        if let s = schema, !s.issues.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                IssueList(model: model, issues: s.issues)
                if s.truncated {
                    Text("Partly checked; validate from this object for everything.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background((s.issues.worst ?? .warning).color.opacity(0.08), in: .rect(cornerRadius: 6))
        }
    }
}

/// For page objects: open the page view.
private struct PageLink: View {
    let model: DocumentModel
    let id: ObjRef

    var body: some View {
        Loaded(model.generation, fetch: { try await model.backend.pageIndex(model.doc, id: id) }) { index in
            if let index {
                LinkButton("Page \(index + 1) of the document →") { model.nav.open(.page(index)) }
                    .font(.caption)
            } else {
                Text("A /Page that the page tree does not reach.").font(.caption).foregroundStyle(.orange)
            }
        }
    }
}

/// Objects referring to `id`, from the inventory's reference graph.
struct Referrers: View {
    let model: DocumentModel
    let id: ObjRef
    @State private var items: [Referrer] = []
    @State private var total: UInt64?
    @State private var error: String?

    private var job: JobInfo? {
        model.latestJob { if case .inventory = $0 { true } else { false } }
    }

    var body: some View {
        if model.summary.inventory == nil {
            HStack(spacing: 6) {
                Text("Referenced by: needs the inventory.").foregroundStyle(.secondary)
                if let job, job.state == .running {
                    Text("Building… \(job.total > 0 ? Int(100 * job.done / job.total) : 0)%")
                        .foregroundStyle(.secondary)
                } else {
                    LinkButton("Build") { Task { try? await model.startJob(.inventory) } }
                }
            }
            .font(.caption)
        } else {
            list
        }
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 2) {
            if let error {
                Text(error).foregroundStyle(.red)
            } else if let total {
                Text(total == 0 ? "Referenced by nothing." : "Referenced by \(total.formatted()):")
                    .foregroundStyle(.secondary)
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(items.enumerated()), id: \.offset) { _, r in
                            HStack(spacing: 6) {
                                LinkButton(r.from.text) { model.nav.open(r.from.target) }.monospaced()
                                if let t = r.typeName { Text(t).foregroundStyle(.teal) }
                                Text(r.paths.joined(separator: "  "))
                                    .monospaced().foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        if UInt64(items.count) < total {
                            LinkButton("Show more (\(items.count.formatted()) of \(total.formatted()))") {
                                Task { await more() }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: min(CGFloat(items.count) * 18 + 24, 140))
                .scrollBounceBehavior(.basedOnSize)
            } else {
                ProgressView().controlSize(.mini)
            }
        }
        .font(.caption)
        .task(id: model.generation) {
            items = []
            total = nil
            await more()
        }
    }

    private func more() async {
        do {
            let p = try await model.backend.referrers(
                model.doc, id: id, range: ListRange(offset: UInt64(items.count), limit: 100))
            items += p.items
            total = p.total
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// The object as written in the file, up to its stream data.
private struct SourceView: View {
    let model: DocumentModel
    let object: ObjectView

    private static let maxSource: UInt64 = 1 << 20

    var body: some View {
        if case .file(let span) = object.location {
            let end = object.stream?.dataSpan?.start ?? span.end
            let len = min(end - span.start, Self.maxSource)
            Loaded(model.generation, fetch: {
                try await model.backend.rawBytes(model.doc, offset: span.start, len: UInt32(len))
            }) { b in
                CodeText(
                    bytesAsText(b.data) + (object.stream != nil ? "\n… stream data …\nendstream\nendobj" : "")
                        + (len < end - span.start ? "\n… truncated" : ""))
            }
        } else {
            Text(
                object.location == .edited
                    ? "Changed by an edit: it is written to the file when the document is saved."
                    : "Stored in an object stream; open the stream to see its decoded data."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .padding(10)
            Spacer()
        }
    }
}
