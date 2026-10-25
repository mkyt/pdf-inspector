import PdfInspectorKit
import SwiftUI

/// Every page, fetched by range, with page operations (the web `PageList`).
struct PagesPanel: View {
    let model: DocumentModel
    @State private var goto = ""
    @State private var adding = false

    var body: some View {
        let _ = Perf.tick("PagesPanel")
        let doc = model.doc
        let backend = model.backend
        let count = model.summary.pages
        VStack(spacing: 0) {
            HStack {
                TextField("Go to page", text: $goto)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        if let n = UInt32(goto.trimmingCharacters(in: .whitespaces)), n >= 1, n <= count {
                            model.nav.open(.page(n - 1))
                        }
                    }
                Text("\(count.formatted()) pages").font(.caption).foregroundStyle(.secondary)
                Button("Add…") { adding = true }
            }
            .controlSize(.small)
            .padding(6)
            Divider()
            PagedRows<PageSummaryPage, _>(
                model: model, total: Int(count), empty: "No pages.",
                fetch: { try await backend.pages(doc, range: $0) }
            ) { p, _ in
                Button {
                    model.nav.open(.page(p.index))
                } label: {
                    HStack {
                        Text(pageText(p.index, label: p.label))
                            .frame(width: 72, alignment: .trailing)
                            .fontWeight(.medium)
                        if let e = p.error {
                            Text(e).foregroundStyle(.red).lineLimit(1).help(e)
                        } else {
                            Text(p.id.map { "\($0.num) \($0.generation)" } ?? "")
                                .monospaced().foregroundStyle(.secondary)
                                .frame(width: 64, alignment: .leading)
                            Text(p.size?.sizeText ?? "—")
                            if p.rotate != 0 { Badge("\(p.rotate)°") }
                            if p.annots > 0 { Badge("\(p.annots)", color: .purple).help("annotations") }
                        }
                        Spacer()
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .font(.callout)
                .contextMenu { PageMenu(model: model, page: p, count: count) }
                .rowBackground(model.nav.place == .page(p.index) ? Color.accentColor.opacity(0.2) : nil)
            }
            .id(count)
        }
        .sheet(isPresented: $adding) { AddPages(model: model) }
    }
}

/// Page operations on one page.
private struct PageMenu: View {
    let model: DocumentModel
    let page: PageSummary
    let count: UInt32
    @Environment(\.inspectorDocument) private var document

    var body: some View {
        let _ = Perf.tick("PageMenu")
        let i = page.index
        let n = pageText(i, label: page.label)
        let pages = [i]
        Text("Page \(n)")
        Button("Rotate Clockwise") { document?.run("Rotate page \(n)", [.rotatePages(pages: pages, by: 90)]) }
        Button("Rotate Counterclockwise") { document?.run("Rotate page \(n)", [.rotatePages(pages: pages, by: -90)]) }
        Divider()
        Button("Move to the Start") { move(0) }.disabled(i == 0)
        Button("Move Up") { move(i - 1) }.disabled(i == 0)
        Button("Move Down") { move(i + 1) }.disabled(i == count - 1)
        Button("Move to the End") { move(count - 1) }.disabled(i == count - 1)
        Divider()
        Button("Duplicate") {
            document?.run("Duplicate page \(n)", [.insertPages(at: i + 1, from: model.doc, pages: pages)])
        }
        Button("Insert a Blank Page Before") { blank(i) }
        Button("Insert a Blank Page After") { blank(i + 1) }
        Divider()
        Button("Delete", role: .destructive) {
            document?.run("Delete page \(n)", [.deletePages(pages: pages)])
            if model.nav.place == .page(i) { model.nav.open(.document) }
        }
    }

    private func move(_ to: UInt32) {
        document?.run("Move page \(pageText(page.index, label: page.label))", [.movePages(pages: [page.index], to: to)])
    }

    private func blank(_ at: UInt32) {
        let w = page.size.map { $0.x1 - $0.x0 } ?? 612
        let h = page.size.map { $0.y1 - $0.y0 } ?? 792
        document?.run("Insert a blank page", [.insertBlankPage(at: at, width: w, height: h)])
    }
}

private let sizes: [(String, Double, Double)] = [
    ("A4", 595.28, 841.89), ("A3", 841.89, 1190.55), ("A5", 419.53, 595.28), ("Letter", 612, 792),
    ("Legal", 612, 1008),
]

/// Blank pages, or pages copied from an open document (this one too).
private struct AddPages: View {
    let model: DocumentModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var copy = false
    @State private var size = "A4"
    @State private var width = 595.28
    @State private var height = 841.89
    @State private var docs: [DocInfo] = []
    @State private var from: DocId?
    @State private var fromPages: UInt32 = 0
    @State private var range = ""
    @State private var at = 1
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        let _ = Perf.tick("AddPages")
        let count = Int(model.summary.pages)
        VStack(alignment: .leading, spacing: 10) {
            Text("Add pages").font(.headline)
            Picker("Source", selection: $copy) {
                Text("Blank page").tag(false)
                Text("From a document").tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            Form {
                if copy {
                    Picker("Document", selection: $from) {
                        Text("An open document").tag(DocId?.none)
                        ForEach(docs, id: \.doc) { d in
                            Text(d.doc == model.doc ? "\(d.name) (this one)" : d.name).tag(Optional(d.doc))
                        }
                    }
                    TextField("Pages", text: $range, prompt: Text(fromPages > 0 ? "1-\(fromPages)" : "all"))
                } else {
                    Picker("Size", selection: $size) {
                        ForEach(sizes, id: \.0) { Text($0.0).tag($0.0) }
                        Text("Custom").tag("Custom")
                    }
                    .onChange(of: size) { _, s in
                        if let p = sizes.first(where: { $0.0 == s }) {
                            width = p.1
                            height = p.2
                        }
                    }
                    TextField("Width (pt)", value: $width, format: .number)
                        .onChange(of: width) { if !sizes.contains(where: { $0.1 == width }) { size = "Custom" } }
                    TextField("Height (pt)", value: $height, format: .number)
                        .onChange(of: height) { if !sizes.contains(where: { $0.2 == height }) { size = "Custom" } }
                }
                Stepper("Before page \(at)", value: $at, in: 1...(count + 1))
                Text("\(count + 1) adds them at the end.").font(.caption).foregroundStyle(.secondary)
            }
            if copy {
                Text(
                    "Copies each page with everything it uses (from this document, the resources are shared). Links to pages not copied become null."
                )
                .font(.caption).foregroundStyle(.secondary)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add") { Task { await submit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || (copy && from == nil))
            }
        }
        .padding()
        .frame(width: 440)
        .task {
            at = count + 1
            docs = (try? await model.backend.docs()) ?? []
        }
        .task(id: from) {
            guard let from else { return }
            fromPages = (try? await model.backend.summary(from).pages) ?? 0
        }
    }

    private func submit() async {
        guard let document else { return }
        let position = UInt32(min(max(at, 1), Int(model.summary.pages) + 1) - 1)
        let command: EditCommand
        if copy {
            guard let from, let pages = parsePages(range.isEmpty ? "1-\(fromPages)" : range, count: fromPages) else {
                error = "Pages are numbers from 1 to \(fromPages), like 1-3, 7."
                return
            }
            command = .insertPages(at: position, from: from, pages: pages)
        } else {
            command = .insertBlankPage(at: position, width: width, height: height)
        }
        busy = true
        defer { busy = false }
        do {
            try await document.perform(copy ? "Insert pages" : "Insert a blank page", [command])
            dismiss()
            model.nav.open(.page(position))
        } catch {
            self.error = error.localizedDescription
        }
    }
}
