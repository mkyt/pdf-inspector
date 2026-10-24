import PdfInspectorKit
import SwiftUI

/// A page: its rendering and what it inherits (the web `PageView` also
/// has tiles, zoom, hit testing, annotations and region cuts).
struct PageView: View {
    let model: DocumentModel
    let index: UInt32
    @State private var detail: PageDetail?
    @State private var image: CGImage?
    @State private var error: String?
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        SplitPane(.horizontal, key: "pageInfo", size: 280, min: 200, max: 500) {
            GeometryReader { geo in
                ScrollView([.horizontal, .vertical]) {
                    if let image {
                        Image(decorative: image, scale: displayScale)
                            .border(.separator)
                            .padding()
                    } else if let error {
                        Text(error).foregroundStyle(.red).padding()
                    } else {
                        ProgressView().padding()
                    }
                }
                .task(id: RenderKey(width: geo.size.width, generation: model.generation)) {
                    await render(width: geo.size.width - 32)
                }
            }
            .frame(minWidth: 300)
        } second: {
            Form {
                if let d = detail {
                    LabeledContent("Page") { Text(pageText(d.index, label: d.label)) }
                    LabeledContent("Object") {
                        Button(d.id.refText) { model.nav.open(.object(d.id)) }.buttonStyle(.link)
                    }
                    LabeledContent("Rotate") { Text("\(d.rotate)°") }
                    ForEach(d.boxes, id: \.name) { b in
                        LabeledContent(b.name) {
                            Text(b.rect.map { "\($0.x0) \($0.y0) \($0.x1) \($0.y1)" } ?? "—").monospaced()
                        }
                    }
                    Button("Operations") { model.nav.open(.content(.page(index: index), op: nil)) }
                }
            }
            .formStyle(.grouped)
        }
        .task(id: model.generation) {
            detail = try? await model.backend.page(model.doc, index: index)
        }
    }

    private struct RenderKey: Equatable {
        let width: Double
        let generation: Int
    }

    private func render(width: Double) async {
        do {
            let content = ContentRef.page(index: index)
            let s = try await model.backend.content(model.doc, content)
            guard s.view.width > 0, width > 0 else { return }
            // Debounce resizing.
            try await Task.sleep(for: .milliseconds(80))
            let scale = width / s.view.width * displayScale
            // TODO: tiles and zoom (RenderView.tsx), instead of one bitmap.
            let b = try await model.backend.render(model.doc, content, scale: min(scale, 8))
            image = b.cgImage()
            error = nil
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}
