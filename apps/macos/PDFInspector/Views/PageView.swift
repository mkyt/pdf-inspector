import PdfInspectorKit
import SwiftUI

/// One page: its rendering, where its attributes come from, its boxes,
/// resources, contents and annotations (the web `PageView`).
struct PageView: View {
    let model: DocumentModel
    let index: UInt32
    /// The annotation selected on the rendering and in the table.
    @State private var annot: UInt32?

    var body: some View {
        let _ = Perf.tick("PageView")
        Loaded(model.generation, fetch: { try await model.backend.page(model.doc, index: index) }) { p in
            SplitPane(.horizontal, key: "pageInfo", size: 420, min: 260, max: 900) {
                VStack(alignment: .leading, spacing: 6) {
                    header(p)
                    Rendering(model: model, index: index, annot: $annot)
                }
                .padding(10)
            } second: {
                ScrollView {
                    PageDetailView(model: model, p: p, annot: $annot)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func header(_ p: PageDetail) -> some View {
        HStack(spacing: 8) {
            Button("Previous Page", systemImage: "chevron.left") { model.nav.open(.page(index - 1)) }
                .disabled(index == 0)
            Text("Page \(pageText(index, label: p.label))").font(.title3.bold())
            Text("of \(model.summary.pages.formatted())").foregroundStyle(.secondary)
            Button("Next Page", systemImage: "chevron.right") { model.nav.open(.page(index + 1)) }
                .disabled(index + 1 >= model.summary.pages)
            Button(p.id.objText) { model.nav.open(.object(p.id)) }
            Button("Operations") { model.nav.open(.content(.page(index: index), op: nil)) }
            Spacer()
        }
        .labelStyle(.iconOnly)
        .controlSize(.small)
    }
}

/// The page rendered; clicking opens the operation painting there.
private struct Rendering: View {
    let model: DocumentModel
    let index: UInt32
    @Binding var annot: UInt32?

    var body: some View {
        let _ = Perf.tick("Rendering")
        let content = ContentRef.page(index: index)
        Loaded(model.generation, fetch: { try await model.backend.content(model.doc, content) }) { s in
            RenderView(
                model: model, content: content, summary: s, op: nil, step: false,
                onPick: { model.nav.open(.content(content, op: $0)) }, annot: $annot)
        }
    }
}

private struct Origin: View {
    let model: DocumentModel
    let o: AttrOrigin

    var body: some View {
        let _ = Perf.tick("Origin")
        switch o {
        case .inherited(let from):
            HStack(spacing: 3) {
                Text("inherited from").foregroundStyle(.secondary)
                ObjLink(model: model, id: from)
            }
        case .missing:
            Text(o.text).foregroundStyle(.red)
        default:
            Text(o.text).foregroundStyle(.secondary)
        }
    }
}

private struct PageDetailView: View {
    let model: DocumentModel
    let p: PageDetail
    @Binding var annot: UInt32?

    var body: some View {
        let _ = Perf.tick("PageDetailView")
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 4) {
                Text("Page tree:").foregroundStyle(.secondary)
                ForEach(Array(p.ancestors.enumerated()), id: \.offset) { i, a in
                    if i > 0 { Text("→").foregroundStyle(.secondary) }
                    ObjLink(model: model, id: a)
                }
            }
            .font(.caption)
            if !p.issues.isEmpty {
                IssueList(model: model, issues: p.issues)
                    .padding(6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background((p.issues.worst ?? .warning).color.opacity(0.08), in: .rect(cornerRadius: 6))
            }
            section("Boxes") {
                Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 2) {
                    ForEach(p.boxes, id: \.name) { b in
                        GridRow {
                            Text("/\(b.name)").monospaced().foregroundStyle(.secondary)
                            Text(b.rect?.text ?? "—").monospaced()
                            Text(b.rect?.sizeText ?? "—")
                            Origin(model: model, o: b.origin)
                        }
                    }
                    GridRow {
                        Text("/Rotate").monospaced().foregroundStyle(.secondary)
                        Text("\(p.rotate)").monospaced().gridCellColumns(2)
                        Origin(model: model, o: p.rotateOrigin)
                    }
                    if p.userUnit != 1 {
                        GridRow {
                            Text("/UserUnit").monospaced().foregroundStyle(.secondary)
                            Text("\(numText(p.userUnit)) (\(numText(p.userUnit / 72)) in per unit)").gridCellColumns(3)
                        }
                    }
                }
            }
            section("Resources") {
                if p.resourceItems.isEmpty {
                    Text("None.").foregroundStyle(.secondary)
                } else {
                    Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 2) {
                        ForEach(Array(p.resourceItems.enumerated()), id: \.offset) { _, r in
                            GridRow {
                                Text("/\(r.category)").monospaced().foregroundStyle(.secondary)
                                Text("/\(r.name)").monospaced()
                                if let id = r.id { ObjLink(model: model, id: id) } else { Text("direct").foregroundStyle(.secondary) }
                                Text(r.detail)
                            }
                        }
                    }
                }
                if p.resourcesTruncated {
                    Text("Only the first \(p.resourceItems.count) are listed.").foregroundStyle(.secondary)
                }
            } right: {
                if let r = p.resources { ObjLink(model: model, id: r) }
                Origin(model: model, o: p.resourcesOrigin)
            }
            section("Contents (\(p.contents.count))") {
                if p.contents.isEmpty {
                    Text("No /Contents: the page is blank.").foregroundStyle(.secondary)
                }
                ForEach(Array(p.contents.enumerated()), id: \.offset) { i, c in
                    HStack(spacing: 8) {
                        Text("[\(i)]").foregroundStyle(.secondary).frame(width: 24, alignment: .leading)
                        if let id = c.id { ObjLink(model: model, id: id) } else { Text("direct") }
                        if let e = c.error {
                            Text(e).foregroundStyle(.red)
                        } else {
                            Text("\(bytesText(c.rawLen ?? 0)) stored" + (c.filters.isEmpty ? "" : " · " + c.filters.joined(separator: ", ")))
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } right: {
                if !p.contents.isEmpty {
                    LinkButton("operations →") { model.nav.open(.content(.page(index: p.index), op: nil)) }
                }
            }
            section("Annotations (\(p.annots.formatted()))") {
                if p.annots > 0 {
                    AnnotationList(model: model, page: p.index, annot: $annot)
                } else {
                    Text("None.").foregroundStyle(.secondary)
                }
            }
        }
        .font(.caption)
        .textSelection(.enabled)
    }

    private func section(
        _ title: String, @ViewBuilder _ content: () -> some View, @ViewBuilder right: () -> some View = { EmptyView() }
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(title).font(.headline)
                right()
            }
            content()
        }
    }
}

private struct AnnotationList: View {
    let model: DocumentModel
    let page: UInt32
    @Binding var annot: UInt32?
    @State private var items: [AnnotInfo] = []
    @State private var total: UInt64?
    @State private var error: String?

    var body: some View {
        let _ = Perf.tick("AnnotationList")
        VStack(alignment: .leading, spacing: 2) {
            ForEach(items, id: \.index) { a in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    LinkButton("[\(a.index)]") { annot = a.index == annot ? nil : a.index }
                        .help("Show it on the page")
                        .frame(width: 36, alignment: .leading)
                    Group {
                        if let id = a.id { ObjLink(model: model, id: id) } else { Text("direct").foregroundStyle(.secondary) }
                    }
                    .frame(width: 80, alignment: .leading)
                    if let e = a.error {
                        Text(e).foregroundStyle(.red)
                    } else {
                        HStack(spacing: 3) {
                            if let c = annotColor(a.color) {
                                Rectangle().fill(c).frame(width: 8, height: 8).border(.black.opacity(0.2))
                            }
                            Badge(a.subtype ?? "?", color: a.shown ? .accentColor : .secondary)
                        }
                        .frame(width: 90, alignment: .leading)
                    }
                    Text(a.rect?.text ?? "").monospaced().foregroundStyle(.secondary)
                    VStack(alignment: .leading) {
                        if let l = a.link { LinkView(model: model, link: l) }
                        Text(describe(a))
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 1)
                .background(a.index == annot ? Color.accentColor.opacity(0.15) : .clear)
            }
            if let error { Text(error).foregroundStyle(.red) }
            if let total, UInt64(items.count) < total {
                LinkButton("Show more (\(items.count.formatted()) of \(total.formatted()))") { Task { await more() } }
            } else if total == nil {
                ProgressView().controlSize(.mini)
            }
        }
        .task(id: model.generation) {
            items = []
            total = nil
            await more()
        }
    }

    private func describe(_ a: AnnotInfo) -> String {
        var s = ""
        if let au = a.author { s += "\(au): " }
        if let c = a.contents { s += c }
        if !a.shown {
            s += " · not shown (" + a.flagNames.filter { ["Invisible", "Hidden", "NoView"].contains($0) }
                .joined(separator: ", ") + ")"
        }
        if let e = a.appearance?.error { s += " · \(e)" }
        if let p = a.parent { s += " · for \(p.objText)" }
        if a.appearance == nil && a.subtype != "Popup" && a.subtype != "Link" { s += " · no appearance" }
        return s
    }

    private func more() async {
        do {
            let p = try await model.backend.annotations(
                model.doc, page: page, range: ListRange(offset: UInt64(items.count), limit: 200))
            items += p.items
            total = p.total
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}
