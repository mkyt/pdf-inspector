import PdfInspectorKit
import SwiftUI

/// The document at a glance: Info (editable), what the catalog declares,
/// XMP metadata (the web `DocumentPanel`).
struct OverviewView: View {
    let model: DocumentModel
    @State private var syncXmp = true
    @State private var info: InfoEdit?
    @State private var xmp = false
    @Environment(\.inspectorDocument) private var document

    var body: some View {
        let _ = Perf.tick("OverviewView")
        Loaded(model.generation, fetch: { try await model.backend.document(model.doc) }) { d in
            form(d)
                .sheet(item: $info) { e in InfoEditor(model: model, edit: e, syncXmp: syncXmp) }
                .sheet(isPresented: $xmp) { XmpEditor(model: model, metadata: d.metadata) }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func form(_ d: DocumentView) -> some View {
        let s = model.summary
        return Form {
            Section {
                if !d.issues.isEmpty {
                    IssueList(model: model, issues: d.issues, withObj: true)
                }
            } header: {
                HStack {
                    Text("Document").font(.title3.bold())
                    Badge("\(d.pages.formatted()) pages")
                    Badge("PDF \(s.pdfVersion ?? s.version ?? "?")")
                    if let c = d.catalog { ObjLink(model: model, id: c) }
                }
            }
            Section {
                if d.infoEntries.isEmpty {
                    Text(d.info == nil ? "No /Info dictionary." : "Empty.").foregroundStyle(.secondary)
                }
                ForEach(d.infoEntries, id: \.key) { e in
                    LabeledContent {
                        HStack(alignment: .firstTextBaseline) {
                            Text("\(e.text)\(Text(e.date.map { " = \($0)" } ?? "").foregroundStyle(.secondary))")
                                .textSelection(.enabled)
                            Button("Edit", systemImage: "pencil") {
                                info = InfoEdit(key: e.key, value: e.date ?? stripName(e.text), fixed: true)
                            }
                            Button("Remove", systemImage: "xmark", role: .destructive) {
                                document?.run(
                                    "Remove /Info /\(e.key)", [.setInfo(key: e.key, value: nil, syncXmp: syncXmp)])
                            }
                        }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                    } label: {
                        Text("/\(e.key)").monospaced()
                    }
                }
            } header: {
                HStack {
                    Text("Info")
                    if let i = d.info { ObjLink(model: model, id: i) }
                    Spacer()
                    Toggle("Update XMP too", isOn: $syncXmp)
                        .help("Changing an Info entry changes the matching XMP property (dc:title, xmp:ModifyDate, …)")
                    Button("Add Entry…") { info = InfoEdit(key: "", value: "", fixed: false) }
                }
                .controlSize(.small)
            }
            Section("Catalog") {
                if d.features.isEmpty { Text("Nothing beyond the page tree.").foregroundStyle(.secondary) }
                ForEach(d.features, id: \.key) { f in
                    LabeledContent {
                        HStack {
                            Text(f.text).textSelection(.enabled)
                            if let obj = f.obj { ObjLink(model: model, id: obj) }
                        }
                    } label: {
                        Text("/\(f.key)").monospaced()
                    }
                }
            }
            Section {
                if d.metadata != nil {
                    XmpTree(model: model)
                } else {
                    Text("The catalog has no /Metadata stream.").foregroundStyle(.secondary)
                }
            } header: {
                HStack {
                    Text("XMP metadata")
                    if let m = d.metadata { ObjLink(model: model, id: m) }
                    Spacer()
                    Button(d.metadata == nil ? "Add XMP…" : "Edit XML…") { xmp = true }
                }
                .controlSize(.small)
            }
        }
        .formStyle(.grouped)
    }

    private func stripName(_ t: String) -> String {
        t.hasPrefix("/") ? String(t.dropFirst()) : t
    }
}

struct ObjLink: View {
    let model: DocumentModel
    let id: ObjRef

    var body: some View {
        let _ = Perf.tick("ObjLink")
        LinkButton(id.objText) { model.nav.open(.object(id)) }
            .font(.caption.monospaced())
    }
}

private struct InfoEdit: Identifiable {
    let id = UUID()
    let key: String
    let value: String
    let fixed: Bool
}

private let infoKeys = [
    "Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate", "Trapped",
]

private struct InfoEditor: View {
    let model: DocumentModel
    let edit: InfoEdit
    let syncXmp: Bool
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var key = ""
    @State private var value = ""
    @State private var error: String?
    @State private var busy = false

    private var k: String {
        let t = key.trimmingCharacters(in: .whitespaces)
        return t.hasPrefix("/") ? String(t.dropFirst()) : t
    }

    var body: some View {
        let _ = Perf.tick("InfoEditor")
        let date = k == "CreationDate" || k == "ModDate"
        VStack(alignment: .leading, spacing: 8) {
            Text(edit.fixed ? "Edit /Info /\(edit.key)" : "Add an Info entry").font(.headline)
            HStack {
                TextField("Key", text: $key)
                    .disabled(edit.fixed)
                if !edit.fixed {
                    Menu("Keys") {
                        ForEach(infoKeys, id: \.self) { k in Button(k) { key = k } }
                    }
                    .fixedSize()
                }
            }
            TextField("Value", text: $value, axis: .vertical)
                .lineLimit(1...8)
            Text(
                date
                    ? "A date: ISO 8601 (2026-10-24T09:30:00+09:00) or a PDF date (D:20261024093000+09'00')."
                    : k == "Trapped"
                        ? "True, False or Unknown (stored as a name)."
                        : "Text; stored as PDFDocEncoding when plain ASCII, UTF-16 otherwise."
            )
            .font(.caption).foregroundStyle(.secondary)
            if date {
                Button("Now") { value = Date.ISO8601FormatStyle(timeZone: .current).format(.now) }.buttonStyle(.inlineLink)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await submit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || k.isEmpty)
            }
        }
        .padding()
        .frame(width: 460)
        .onAppear {
            key = edit.key
            value = edit.value
        }
    }

    private func submit() async {
        guard let document else { return }
        busy = true
        defer { busy = false }
        do {
            try await document.perform("Set /Info /\(k)", [.setInfo(key: k, value: value, syncXmp: syncXmp)])
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private let xmpTemplate = """
    <?xpacket begin="\u{FEFF}" id="W5M0MpCehiHzreSzNTczkc9d"?>
    <x:xmpmeta xmlns:x="adobe:ns:meta/">
      <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
        <rdf:Description rdf:about=""
            xmlns:dc="http://purl.org/dc/elements/1.1/"
            xmlns:xmp="http://ns.adobe.com/xap/1.0/"
            xmlns:pdf="http://ns.adobe.com/pdf/1.3/">
        </rdf:Description>
      </rdf:RDF>
    </x:xmpmeta>
    <?xpacket end="w"?>
    """

private struct XmpEditor: View {
    let model: DocumentModel
    let metadata: ObjRef?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var text: String?
    @State private var error: String?
    @State private var busy = false

    /// Largest metadata loaded for editing as text.
    private static let maxXmp: UInt32 = 4 << 20

    var body: some View {
        let _ = Perf.tick("XmpEditor")
        VStack(alignment: .leading, spacing: 8) {
            Text("XMP metadata").font(.headline)
            if text == nil && error == nil {
                ProgressView().frame(maxWidth: .infinity, minHeight: 300)
            } else {
                CodeText(editing: Binding { text ?? "" } set: { text = $0 })
                    .frame(minHeight: 320)
                    .border(.separator)
            }
            Text("Stored unfiltered (UTF-8), as the catalog’s /Metadata stream. It must be well-formed XML.")
                .font(.caption).foregroundStyle(.secondary)
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await submit() } }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(busy || text == nil)
            }
        }
        .padding()
        .frame(minWidth: 680, minHeight: 460)
        .task {
            guard let metadata else {
                text = xmpTemplate
                return
            }
            do {
                let b = try await model.backend.streamData(
                    model.doc, id: metadata, decoded: true, offset: 0, len: Self.maxXmp)
                if !b.info.eof { error = "The metadata is too large to edit here." }
                text = String(decoding: b.data, as: UTF8.self)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func submit() async {
        guard let document, let text else { return }
        busy = true
        defer { busy = false }
        do {
            try await document.perform(
                metadata == nil ? "Add XMP metadata" : "Edit the XMP metadata", [.setXmp(xml: text)])
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// XMP properties and the tree, page by page.
private struct XmpTree: View {
    let model: DocumentModel
    @State private var first: XmpView?
    @State private var nodes: [XmpNode] = []
    @State private var error: String?

    var body: some View {
        let _ = Perf.tick("XmpTree")
        VStack(alignment: .leading, spacing: 6) {
            if let error {
                Text(error).foregroundStyle(.red)
            } else if let f = first {
                Text("\(bytesText(f.len)) · \(f.nodes.total.formatted()) nodes\(f.truncated ? " (tree truncated)" : "")")
                    .font(.caption).foregroundStyle(.secondary)
                if let e = f.error { Text(e).font(.caption).foregroundStyle(.orange) }
                if !f.properties.isEmpty {
                    Grid(alignment: .topLeading, horizontalSpacing: 10, verticalSpacing: 2) {
                        ForEach(Array(f.properties.enumerated()), id: \.offset) { _, p in
                            GridRow {
                                Text(p.name).monospaced().foregroundStyle(.secondary).help(p.namespace)
                                Text(p.value).textSelection(.enabled)
                            }
                        }
                    }
                    .font(.caption)
                }
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(Array(nodes.enumerated()), id: \.offset) { _, n in
                            Text(
                                "\(Text(n.kind == .attribute ? "@\(n.name)" : "<\(n.name)>").foregroundStyle(n.kind == .attribute ? Color.purple : Color.blue))\(n.value.map { " \($0)" } ?? "")"
                            )
                            .padding(.leading, CGFloat(n.depth) * 14)
                            .help(n.namespace ?? "")
                        }
                    }
                    .font(.system(size: 11, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 420)
                if UInt64(nodes.count) < f.nodes.total {
                    LinkButton("Show more (\(nodes.count.formatted()) of \(f.nodes.total.formatted()))") {
                        Task { await more() }
                    }
                    .font(.caption)
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .task(id: model.generation) {
            first = nil
            nodes = []
            await more()
        }
    }

    private func more() async {
        do {
            let v = try await model.backend.xmp(model.doc, range: ListRange(offset: UInt64(nodes.count), limit: 500))
            if first == nil { first = v }
            nodes += v.nodes.items
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}
