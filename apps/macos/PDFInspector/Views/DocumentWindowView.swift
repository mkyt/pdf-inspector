import PdfInspectorKit
import SwiftUI

/// A document window (the web `DocView`): the sidebar with the document's
/// lists, the current target in the middle, the file bytes below it and the
/// schema panel on the right.
struct DocumentWindowView: View {
    let model: DocumentModel
    @Bindable var layout: WindowLayout

    var body: some View {
        let _ = Perf.tick("DocumentWindowView")
        NavigationSplitView(columnVisibility: sidebarVisibility) {
            Sidebar(model: model, layout: layout)
                .navigationSplitViewColumnWidth(min: 260, ideal: 380, max: 700)
        } detail: {
            SplitPane(.vertical, key: "hex", size: 280, min: 100, max: 1200, showSecond: layout.bytes) {
                VStack(spacing: 0) {
                    Breadcrumbs(model: model)
                    Divider()
                    TargetView(model: model)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } second: {
                HexView(model: model)
            }
            // A fixed minimum: the split view lays out again whenever the
            // detail's minimum changes, and what it shows changes with it.
            .frame(minWidth: 360, maxWidth: .infinity, minHeight: 300, maxHeight: .infinity)
            .clipped()
        }
        .inspector(isPresented: $layout.schema) {
            SchemaPane(model: model)
                .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
                .inspectorColumnWidth(min: 200, ideal: 300, max: 800)
        }
        .toolbar { DocumentToolbar(model: model, layout: layout) }
        .overlay(alignment: .bottomLeading) { MessagesOverlay(model: model) }
        .onChange(of: model.nav.bytesRequest) { layout.bytes = true }
        .sheet(isPresented: $layout.reopening) { ReopenSheet(model: model) }
        .frame(minWidth: 800, minHeight: 500)
    }

    private var sidebarVisibility: Binding<NavigationSplitViewVisibility> {
        Binding {
            layout.sidebar ? .all : .detailOnly
        } set: {
            layout.sidebar = $0 != .detailOnly
        }
    }
}

/// What the middle of the window shows for the current target.
struct TargetView: View {
    let model: DocumentModel

    var body: some View {
        let _ = Perf.tick("TargetView")
        switch model.nav.target {
        case .document:
            OverviewView(model: model)
        case .trailer:
            InspectorView(model: model, root: .trailer)
        case .object(let id):
            InspectorView(model: model, root: .object(id: id))
        case .page(let index):
            PageView(model: model, index: index)
                .id(index)
        case .content(let content, let op):
            ContentView(model: model, content: content, op: op)
                .id(content)
        case .font(let id):
            FontView(model: model, id: id)
                .id(id)
        }
    }
}

struct DocumentToolbar: ToolbarContent {
    let model: DocumentModel
    let layout: WindowLayout
    @Environment(\.inspectorDocument) private var document

    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .navigation) {
            Button("Back", systemImage: "chevron.left") { model.nav.goBack() }
                .disabled(!model.nav.canGoBack)
            Button("Forward", systemImage: "chevron.right") { model.nav.goForward() }
                .disabled(!model.nav.canGoForward)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            if model.edit.dirty {
                Text("\(model.edit.editedObjects) edited")
                    .font(.caption)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(.yellow.opacity(0.25), in: .capsule)
                    .help("Objects added, changed or deleted and not saved")
            }
            Button("File Bytes", systemImage: "rectangle.bottomhalf.inset.filled") {
                layout.bytes.toggle()
            }
            .help("Show or hide the file bytes (⌘J)")
            Button("Schema", systemImage: "sidebar.right") { layout.schema.toggle() }
                .help("Show or hide the schema panel (⌥⌘B)")
        }
    }
}

extension Target {
    /// The number of the object shown, if it is one.
    var objectNum: UInt32? {
        if case .object(let id) = self { id.num } else { nil }
    }

    func title(in s: DocSummary) -> String {
        switch self {
        case .document: "Overview"
        case .trailer: "Trailer"
        case .object(let id): id == s.root ? "Catalog" : id.objText
        case .page(let i): "Page \(i + 1)"
        case .content(.page(let i), _): "Page \(i + 1) · operations"
        case .content(.stream(let id), _): "\(id.objText) · operations"
        case .font(let id): "Font \(id.refText)"
        }
    }
}
