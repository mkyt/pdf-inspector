import PdfInspectorKit
import SwiftUI

/// Rows of a `PagedList`, fetched as they scroll into view (the web
/// `PagedList`). The list is dropped and fetched again when the document
/// changes.
struct PagedRows<P: ListPage, Row: View>: View {
    let model: DocumentModel
    let empty: String
    @State private var list: PagedList<P>
    private let row: (P.Item, Int) -> Row

    init(
        model: DocumentModel, total: Int, empty: String = "Nothing here.",
        fetch: @escaping @Sendable (ListRange) async throws -> P,
        @ViewBuilder row: @escaping (P.Item, Int) -> Row
    ) {
        self.model = model
        self.empty = empty
        _list = State(initialValue: PagedList(total: total, fetch: fetch))
        self.row = row
    }

    var body: some View {
        Group {
            if list.total == 0 {
                Text(empty)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(0..<list.total, id: \.self) { i in
                    Group {
                        if let item = list.item(i) {
                            row(item, i)
                        } else {
                            ProgressView().controlSize(.mini)
                        }
                    }
                    .onAppear { list.need(i) }
                }
                .listStyle(.plain)
            }
        }
        .overlay(alignment: .bottom) {
            if let e = list.error {
                Text(e).font(.caption).foregroundStyle(.red).padding(4)
            }
        }
        .onChange(of: model.generation) { list.reset() }
    }
}

extension XrefEntry {
    var generation: UInt16 {
        switch self {
        case .free(_, let g), .inUse(_, let g), .edited(let g): g
        case .compressed: 0
        }
    }
}

struct DiagnosticsPanel: View {
    let model: DocumentModel

    var body: some View {
        let doc = model.doc
        let backend = model.backend
        PagedRows<DiagnosticInfoPage, _>(
            model: model, total: Int(model.summary.diagnostics), empty: "No diagnostics.",
            fetch: { try await backend.diagnostics(doc, range: $0) }
        ) { d, _ in
            DiagnosticRow(d: d) { model.nav.open(.object($0)) }
        }
    }
}

struct DiagnosticRow: View {
    let d: DiagnosticInfo
    let open: (ObjRef) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Image(systemName: d.severity.symbol).foregroundStyle(d.severity.color)
            VStack(alignment: .leading) {
                Text(d.message)
                HStack {
                    Text(d.code).monospaced()
                    if let obj = d.obj {
                        Button(obj.refText) { open(obj) }.buttonStyle(.link)
                    }
                    if let p = d.path { Text(p).monospaced() }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .font(.callout)
    }
}

extension Severity {
    var symbol: String {
        switch self {
        case .info: "info.circle"
        case .warning: "exclamationmark.triangle"
        case .error: "xmark.octagon"
        }
    }

    var color: Color {
        switch self {
        case .info: .blue
        case .warning: .orange
        case .error: .red
        }
    }
}

/// Lists that need the inventory job first (the web `InventoryGate`).
struct InventoryGate<Content: View>: View {
    let model: DocumentModel
    @ViewBuilder let content: (InventoryStats) -> Content
    @State private var error: String?

    private var job: JobInfo? {
        model.latestJob { if case .inventory = $0 { true } else { false } }
    }

    var body: some View {
        if let stats = model.summary.inventory {
            content(stats)
        } else {
            VStack(spacing: 8) {
                Text("Fonts, images and references need a pass over every object and page.")
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
                Button("Build inventory") {
                    Task {
                        do { try await model.startJob(.inventory) } catch { self.error = error.localizedDescription }
                    }
                }
                .disabled(job?.state == .running)
                if let job { JobProgress(job: job).frame(width: 200) }
                if let error { Text(error).foregroundStyle(.red) }
            }
            .padding()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

struct FontsPanel: View {
    let model: DocumentModel

    var body: some View {
        let doc = model.doc
        let backend = model.backend
        InventoryGate(model: model) { stats in
            PagedRows<FontInfoPage, _>(
                model: model, total: Int(stats.fonts), empty: "No fonts.",
                fetch: { try await backend.fonts(doc, range: $0) }
            ) { f, _ in
                Button {
                    model.nav.open(.font(f.id))
                } label: {
                    HStack {
                        Text(f.baseFont ?? "(no name)").lineLimit(1)
                        Text(f.subtype ?? "").foregroundStyle(.secondary)
                        Spacer()
                        Text(f.embedded ?? "not embedded").foregroundStyle(.secondary)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .font(.callout)
            }
        }
    }
}

struct ImagesPanel: View {
    let model: DocumentModel

    var body: some View {
        let doc = model.doc
        let backend = model.backend
        InventoryGate(model: model) { stats in
            PagedRows<ImageInfoPage, _>(
                model: model, total: Int(stats.images), empty: "No images.",
                fetch: { try await backend.images(doc, range: $0) }
            ) { im, _ in
                Button {
                    model.nav.open(.object(im.id))
                } label: {
                    HStack {
                        Text(im.id.refText).monospaced()
                        Text("\(im.width ?? 0) × \(im.height ?? 0)")
                        Text(im.colorSpace ?? "").foregroundStyle(.secondary)
                        Spacer()
                        Text(bytesText(im.rawLen)).foregroundStyle(.secondary)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .font(.callout)
            }
        }
    }
}
