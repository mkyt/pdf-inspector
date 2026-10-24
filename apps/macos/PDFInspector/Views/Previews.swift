#if DEBUG
import PdfInspectorKit
import SwiftUI

// Previews of a document window on the test fixtures. The app is
// sandboxed, so they are read from its container:
//
//   mkdir -p ~/Library/Containers/com.juzbox.pdf-inspector/Data/Library/Caches/fixtures
//   cp tests/fixtures/*.pdf ~/Library/Containers/com.juzbox.pdf-inspector/Data/Library/Caches/fixtures

@MainActor
private func previewModel(_ fixture: String, _ target: Target) -> DocumentModel {
    let path = URL.cachesDirectory.appending(path: "fixtures/\(fixture)").path
    let reply = try! Backend.shared.sendNow(.open(source: .path(path: path), password: nil, recover: false))
    guard case .doc(let s) = reply.body else { fatalError("\(reply.body)") }
    let model = DocumentModel(summary: s)
    model.nav.open(target)
    return model
}

@MainActor
private func previewWindow(
    _ fixture: String, _ target: Target, schema: Bool = true, bytes: Bool = true, tab: SidebarTab = .pages
) -> some View {
    let layout = WindowLayout()
    layout.schema = schema
    layout.bytes = bytes
    layout.tab = tab
    return DocumentWindowView(model: previewModel(fixture, target), layout: layout)
        .frame(width: 1400, height: 860)
}

#Preview("Object") {
    previewWindow("schema.pdf", .object(ObjRef(num: 2, generation: 0)))
}

#Preview("Stream") {
    previewWindow("model.pdf", .object(ObjRef(num: 10, generation: 0)), tab: .objects)
}

#Preview("Overview") {
    previewWindow("schema.pdf", .document, bytes: false)
}

#Preview("Font") {
    FontView(model: previewModel("fonts.pdf", .document), id: ObjRef(num: 21, generation: 0))
        .frame(width: 900, height: 800)
}

#Preview("Overview pane") {
    OverviewView(model: previewModel("model.pdf", .document))
        .frame(width: 900, height: 800)
}
#endif
