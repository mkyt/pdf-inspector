import AppKit
import PdfInspectorKit
import SwiftUI

/// Rows of a `PagedList`, fetched as they scroll into view (the web
/// `PagedList`). The list is dropped and fetched again when the document
/// changes. Rows are `rowHeight` high, or as high as they need when `fits`
/// (measured as they are shown); `rowBackground` colors a whole row.
struct PagedRows<P: ListPage, Row: View>: View {
    let model: DocumentModel
    let empty: String
    let rowHeight: Double
    let fits: Bool
    @State private var list: PagedList<P>
    private let row: (P.Item, Int) -> Row

    init(
        model: DocumentModel, total: Int, empty: String = "Nothing here.", rowHeight: Double = 24, fits: Bool = false,
        fetch: @escaping @Sendable (ListRange) async throws -> P,
        @ViewBuilder row: @escaping (P.Item, Int) -> Row
    ) {
        self.model = model
        self.empty = empty
        self.rowHeight = rowHeight
        self.fits = fits
        _list = State(initialValue: PagedList(total: total, fetch: fetch))
        self.row = row
    }

    var body: some View {
        let _ = Perf.tick("PagedRows")
        Group {
            if list.total == 0 {
                Text(empty)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                PagedTable(list: list, version: list.version, rowHeight: rowHeight, fits: fits, row: row)
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

extension View {
    /// The background of the whole row of a `PagedRows` (like
    /// `listRowBackground`).
    func rowBackground(_ color: Color?) -> some View {
        preference(key: RowBackground.self, value: color)
    }
}

private struct RowBackground: PreferenceKey {
    static var defaultValue: Color? { nil }
    static func reduce(value: inout Color?, nextValue: () -> Color?) { value = value ?? nextValue() }
}

/// The rows of a `PagedRows` in an NSTableView, which asks only for the rows
/// on screen (SwiftUI's `List` diffs every row on each update: seconds for
/// 100,000 objects). Each row is a SwiftUI view of its own: what it reads
/// (the window's place) updates the rows on screen only.
private struct PagedTable<P: ListPage, Row: View>: NSViewRepresentable {
    let list: PagedList<P>
    /// `list.version`, read in the owner's body so that it redraws.
    let version: Int
    let rowHeight: Double
    let fits: Bool
    let row: (P.Item, Int) -> Row

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
        let column = NSTableColumn(identifier: .init("row"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.rowHeight = rowHeight
        table.intercellSpacing = .zero
        table.selectionHighlightStyle = .none
        table.allowsEmptySelection = true
        table.focusRingType = .none
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        context.coordinator.table = table
        context.coordinator.shown = self
        table.reloadData()
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.update(self)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var shown: PagedTable?
        weak var table: NSTableView?
        /// Heights of rows measured (with `fits`), by row; for the width.
        private var heights: [Int: CGFloat] = [:]
        private var measuredWidth: CGFloat = 0

        func update(_ p: PagedTable) {
            guard let table, let old = shown else { return }
            shown = p
            if old.list !== p.list || table.numberOfRows != p.list.total {
                heights = [:]
                table.reloadData()
                return
            }
            // The rows' closures are new: show the rows on screen with them.
            let rows = table.rows(in: table.visibleRect)
            guard rows.length > 0 else { return }
            Perf.tick("PagedTable.reloadRows")
            table.reloadData(forRowIndexes: IndexSet(integersIn: rows.location..<rows.location + rows.length), columnIndexes: [0])
        }

        func numberOfRows(in tableView: NSTableView) -> Int { shown?.list.total ?? 0 }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let p = shown else { return nil }
            Perf.tick("PagedTable.viewFor")
            p.list.need(row)
            let content = PagedCell(item: p.list.item(row), index: row, row: p.row)
            let id = NSUserInterfaceItemIdentifier("PagedCell")
            let cell: NSHostingView<PagedCell<P.Item, Row>>
            if let c = tableView.makeView(withIdentifier: id, owner: nil) as? NSHostingView<PagedCell<P.Item, Row>> {
                c.rootView = content
                cell = c
            } else {
                cell = NSHostingView(rootView: content)
                cell.identifier = id
                cell.sizingOptions = []
            }
            if p.fits, content.item != nil { measure(content, row: row, in: tableView) }
            return cell
        }

        /// Sizes rows for `fits`.
        private var measurer: NSHostingController<PagedCell<P.Item, Row>>?

        private func measure(_ content: PagedCell<P.Item, Row>, row: Int, in tableView: NSTableView) {
            guard let p = shown else { return }
            let width = tableView.tableColumns.first?.width ?? tableView.bounds.width
            if width != measuredWidth {
                measuredWidth = width
                heights = [:]
            }
            guard heights[row] == nil else { return }
            let m = measurer ?? NSHostingController(rootView: content)
            measurer = m
            m.rootView = content
            let h = max(p.rowHeight, m.sizeThatFits(in: NSSize(width: width, height: 10_000)).height.rounded(.up))
            heights[row] = h
            if h != p.rowHeight {
                // After this row's view is in place.
                DispatchQueue.main.async { tableView.noteHeightOfRows(withIndexesChanged: [row]) }
            }
        }

        func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
            heights[row] ?? shown?.rowHeight ?? 24
        }

        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }
    }
}

/// A row of a `PagedTable`: the item's row, or a spinner while its page
/// loads; the row is made here so that what it reads updates it.
private struct PagedCell<Item, Row: View>: View {
    let item: Item?
    let index: Int
    let row: (Item, Int) -> Row

    var body: some View {
        let _ = Perf.tick("PagedCell")
        Group {
            if let item {
                row(item, index)
            } else {
                ProgressView().controlSize(.mini)
            }
        }
        .padding(.horizontal, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .backgroundPreferenceValue(RowBackground.self) { $0 ?? .clear }
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
        let _ = Perf.tick("DiagnosticsPanel")
        let doc = model.doc
        let backend = model.backend
        PagedRows<DiagnosticInfoPage, _>(
            model: model, total: Int(model.summary.diagnostics), empty: "No diagnostics.", fits: true,
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
        let _ = Perf.tick("DiagnosticRow")
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
        let _ = Perf.tick("InventoryGate")
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
        let _ = Perf.tick("FontsPanel")
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
        let _ = Perf.tick("ImagesPanel")
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
