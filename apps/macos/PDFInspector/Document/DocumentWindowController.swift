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
}

/// A document window: the SwiftUI `DocumentWindowView` in AppKit, with the menu
/// commands of the window (navigation, panes) on the responder chain.
final class DocumentWindowController: NSWindowController, NSMenuItemValidation {
    let model: DocumentModel
    let layout: WindowLayout

    init(document: InspectorDocument, model: DocumentModel) {
        let layout = WindowLayout()
        self.model = model
        self.layout = layout
        let root = DocumentWindowView(model: model, layout: layout)
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
        // Where the window is, under the document's name.
        // TODO: breadcrumbs (Breadcrumbs.tsx).
        Task { [weak window] in
            for await t in Observations({ model.nav.target.title(in: model.summary) }) {
                window?.subtitle = t
            }
        }
    }

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
