import AppKit
import PdfInspectorKit
import SwiftUI

/// Which panes of a document window are shown (the web `layout.ts`).
@MainActor @Observable
final class WindowLayout {
    var sidebar = true
    var schema = false
    var bytes = false
    var tab: SidebarTab = .pages
    var objectPicks = ObjectPicks()
    /// The sheet reopening the file with a password or recovery is shown.
    var reopening = false
}

/// The document as the window shows it: replaced when the file is reopened
/// (with a password, or recovered).
@MainActor @Observable
final class ModelBox {
    var model: DocumentModel

    init(_ model: DocumentModel) {
        self.model = model
    }
}

private struct DocumentRoot: View {
    let box: ModelBox
    let layout: WindowLayout

    var body: some View {
        DocumentWindowView(model: box.model, layout: layout)
            .id(ObjectIdentifier(box.model))
    }
}

/// A document window: the SwiftUI `DocumentWindowView` in AppKit, with the menu
/// commands of the window (navigation, panes) on the responder chain.
final class DocumentWindowController: NSWindowController, NSMenuItemValidation {
    let box: ModelBox
    let layout: WindowLayout
    var model: DocumentModel { box.model }

    init(document: InspectorDocument, box: ModelBox) {
        let layout = WindowLayout()
        self.box = box
        self.layout = layout
        let root = DocumentRoot(box: box, layout: layout)
            .environment(\.inspectorDocument, document)
        let hosting = NSHostingController(rootView: root)
        hosting.sceneBridgingOptions = [.toolbars]
        let window = NSWindow(contentViewController: hosting)
        window.styleMask.insert(.fullSizeContentView)
        window.setContentSize(NSSize(width: 1280, height: 820))
        window.setFrameAutosaveName("DocumentWindow")
        window.tabbingMode = .preferred
        super.init(window: window)
        shouldCascadeWindows = true
        // Where the window is, under the document's name (the breadcrumbs
        // tell the whole path).
        Task { [weak window] in
            for await t in Observations({ box.model.nav.target.title(in: box.model.summary) }) {
                window?.subtitle = t
            }
        }
    }

    @objc func reopenWithOptions(_ sender: Any?) { layout.reopening = true }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a nib")
    }

    @objc func goBack(_ sender: Any?) { model.nav.goBack() }
    @objc func goForward(_ sender: Any?) { model.nav.goForward() }
    @objc func showOverview(_ sender: Any?) { model.nav.open(.document) }
    @objc func showTrailer(_ sender: Any?) { model.nav.open(.trailer) }

    @objc func showCatalog(_ sender: Any?) {
        if let root = model.summary.root { model.nav.open(.object(root)) }
    }

    @objc func toggleSidebarPane(_ sender: Any?) {
        layout.sidebar.toggle()
    }

    @objc func toggleSchemaPane(_ sender: Any?) { layout.schema.toggle() }
    @objc func toggleBytesPane(_ sender: Any?) { layout.bytes.toggle() }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(goBack(_:)): model.nav.canGoBack
        case #selector(goForward(_:)): model.nav.canGoForward
        case #selector(showCatalog(_:)): model.summary.root != nil
        default: true
        }
    }
}

extension EnvironmentValues {
    /// The document of the window, for edits (they go through its undo
    /// manager).
    @Entry var inspectorDocument: InspectorDocument?
}
