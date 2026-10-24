import AppKit

/// The main menu, built in code (no nib). Items act through the responder
/// chain: NSDocument handles the file commands, the document's window
/// controller the navigation and panes.
@MainActor
enum MainMenu {
    static func make() -> NSMenu {
        let main = NSMenu()
        let name = ProcessInfo.processInfo.processName

        main.addSubmenu("", [
            item("About \(name)", #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
            .separator(),
            item("Hide \(name)", #selector(NSApplication.hide(_:)), "h"),
            item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            item("Show All", #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item("Quit \(name)", #selector(NSApplication.terminate(_:)), "q"),
        ])

        let recent = NSMenu(title: "Open Recent")
        recent.addItem(item("Clear Menu", #selector(NSDocumentController.clearRecentDocuments(_:))))
        let recentItem = NSMenuItem(title: "Open Recent", action: nil, keyEquivalent: "")
        recentItem.submenu = recent
        main.addSubmenu("File", [
            item("Open…", #selector(NSDocumentController.openDocument(_:)), "o"),
            recentItem,
            .separator(),
            item("Close", #selector(NSWindow.performClose(_:)), "w"),
            item("Save", #selector(NSDocument.save(_:)), "s"),
            item("Save As…", #selector(NSDocument.saveAs(_:)), "s", [.command, .shift]),
            item("Revert to Saved", #selector(NSDocument.revertToSaved(_:))),
            item("Reopen with Options…", #selector(DocumentWindowController.reopenWithOptions(_:))),
        ])

        main.addSubmenu("Edit", [
            item("Undo", Selector(("undo:")), "z"),
            item("Redo", Selector(("redo:")), "z", [.command, .shift]),
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ])

        main.addSubmenu("View", [
            item("Toggle Sidebar", #selector(DocumentWindowController.toggleSidebarPane(_:)), "b"),
            item("Toggle Schema Panel", #selector(DocumentWindowController.toggleSchemaPane(_:)), "b", [.command, .option]),
            item("Toggle File Bytes", #selector(DocumentWindowController.toggleBytesPane(_:)), "j"),
            .separator(),
            item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", [.command, .control]),
        ])

        main.addSubmenu("Go", [
            item("Back", #selector(DocumentWindowController.goBack(_:)), "["),
            item("Forward", #selector(DocumentWindowController.goForward(_:)), "]"),
            .separator(),
            item("Overview", #selector(DocumentWindowController.showOverview(_:)), "1"),
            item("Trailer", #selector(DocumentWindowController.showTrailer(_:)), "2"),
            item("Catalog", #selector(DocumentWindowController.showCatalog(_:)), "3"),
        ])

        let window = main.addSubmenu("Window", [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:))),
            .separator(),
            item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))),
        ])
        NSApplication.shared.windowsMenu = window
        return main
    }

    private static func item(
        _ title: String, _ action: Selector, _ key: String = "",
        _ modifiers: NSEvent.ModifierFlags = .command
    ) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.keyEquivalentModifierMask = modifiers
        return i
    }
}

extension NSMenu {
    @discardableResult
    fileprivate func addSubmenu(_ title: String, _ items: [NSMenuItem]) -> NSMenu {
        let menu = NSMenu(title: title)
        items.forEach(menu.addItem)
        let top = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        top.submenu = menu
        addItem(top)
        return menu
    }
}
