import AppKit
import PdfInspectorKit

/// AppKit lifecycle: documents are `InspectorDocument`s (NSDocument reads by
/// URL, so the file is never loaded whole), each window hosts SwiftUI.
@main
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private static let delegate = AppDelegate()

    static func main() {
        let app = NSApplication.shared
        app.delegate = delegate
        app.mainMenu = MainMenu.make()
        app.run()
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if UserDefaults.standard.string(forKey: "PerfBench") != nil {
            PerfBench.runIfAsked()
            return
        }
        if Perf.enabled { MainThreadMonitor.shared.start() }
        // Nothing reopened or opened from the Finder: ask for a file.
        DispatchQueue.main.async {
            if NSDocumentController.shared.documents.isEmpty {
                NSDocumentController.shared.openDocument(nil)
            }
        }
    }
}
