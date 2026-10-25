import AppKit
import PdfInspectorKit
import SwiftUI
import UniformTypeIdentifiers

/// What the preview reads of an image XObject's dictionary.
struct ImageDict: Equatable {
    var width: Int
    var height: Int
    var bits: Int
    /// Device color space name, if it is one.
    var space: String?
    var mask: Bool

    init?(_ nodes: [NodeView]) {
        func get(_ k: String) -> Value? { nodes.first { $0.key == k }?.value }
        func int(_ k: String) -> Int? { if case .integer(let v) = get(k) { Int(v) } else { nil } }
        guard case .name("Image") = get("Subtype"), let w = int("Width"), let h = int("Height"), w > 0, h > 0
        else { return nil }
        width = w
        height = h
        bits = int("BitsPerComponent") ?? 1
        if case .name(let n) = get("ColorSpace") { space = n }
        if case .bool(true) = get("ImageMask") { mask = true } else { mask = false }
    }
}

private let plainFilters: Set<String> = [
    "FlateDecode", "LZWDecode", "ASCII85Decode", "ASCIIHexDecode", "RunLengthDecode",
]

extension ImageFile {
    var label: String {
        switch self {
        case .jpeg: "JPEG"
        case .jpeg2000: "JPEG 2000"
        }
    }
    var ext: String {
        switch self {
        case .jpeg: "jpg"
        case .jpeg2000: "jp2"
        }
    }
    var type: UTType {
        switch self {
        case .jpeg: .jpeg
        case .jpeg2000: UTType("public.jpeg-2000") ?? .data
        }
    }
}

/// A preview of an image XObject (the web `ImagePreview`) as the renderer
/// decodes it: `/Decode`, color spaces and tint transforms, soft masks.
/// Saves it as PNG, as the stored JPEG or JPEG 2000 file when viewers show
/// that the same way, or as its data.
struct ImagePreview: View {
    let model: DocumentModel
    let id: ObjRef
    let info: StreamInfo
    @State private var dict: ImageDict?
    @State private var decoded: DecodedImage?
    @State private var image: CGImage?
    @State private var note: String?
    @State private var error: String?

    private static let maxSide = 320.0

    var body: some View {
        let _ = Perf.tick("ImagePreview")
        if let d = dict {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text("Image").fontWeight(.semibold)
                    Text("\(d.width) × \(d.height) · \(d.mask ? "ImageMask" : d.space ?? "other color space") · \(d.bits) bpc")
                        .foregroundStyle(.secondary)
                    if let f = decoded?.file {
                        Button("Save \(f.label)…") { save(as: .stored(f.ext, f.type)) }
                    }
                    if decoded?.part != nil {
                        Button("Save PNG…") { save(as: .png) }
                    }
                    Button(samplesLabel) { save(as: .stored("bin", .data)) }
                }
                .buttonStyle(.inlineLink)
                if let image {
                    // Large images shrink to fit; small ones grow (up to 32×).
                    let scale = min(32, Self.maxSide / Double(max(d.width, d.height)))
                    Image(decorative: image, scale: 1)
                        .resizable()
                        .interpolation(scale > 1 ? .none : .medium)
                        .frame(width: Double(d.width) * scale, height: Double(d.height) * scale)
                        .background(Checkerboard())
                        .border(.separator)
                } else if let note {
                    Text(note).foregroundStyle(.secondary)
                } else {
                    ProgressView().controlSize(.mini)
                }
                if let error { Text(error).foregroundStyle(.red) }
            }
            .font(.caption)
            .task(id: model.generation) { await load(d) }
        } else {
            Color.clear.frame(height: 0)
                .task(id: model.generation) {
                    let nodes = try? await model.backend.children(
                        model.doc, path: NodePath(root: .object(id: id), path: []),
                        range: ListRange(offset: 0, limit: 200))
                    dict = nodes.flatMap { ImageDict($0.items) }
                }
        }
    }

    private var samplesLabel: String {
        if let last = info.filters.last, !plainFilters.contains(last) { "Save data (/\(last))…" } else { "Save samples…" }
    }

    private func load(_ d: ImageDict) async {
        // Decoded at about twice the preview's size.
        let reduce = max(1, max(d.width, d.height) / Int(2 * Self.maxSide))
        do {
            let r = try await model.backend.decodeImage(model.doc, id: id, reduce: UInt32(reduce))
            decoded = r.info
            image = rgbaImage(width: Int(r.info.width), height: Int(r.info.height), data: r.data)
            note = image == nil ? r.info.error ?? "No preview for this image." : nil
        } catch is CancellationError {
        } catch {
            note = error.localizedDescription
        }
    }

    private enum SaveKind {
        case png
        case stored(String, UTType)
    }

    private func save(as kind: SaveKind) {
        let panel = NSSavePanel()
        let stem = (model.summary.info.name as NSString).deletingPathExtension
        let ext: String
        switch kind {
        case .png:
            ext = "png"
            panel.allowedContentTypes = [.png]
        case .stored(let e, let t):
            ext = e
            panel.allowedContentTypes = [t]
        }
        panel.nameFieldStringValue = "\(stem)-\(id.num)-\(id.generation).\(ext)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task {
            do {
                switch kind {
                case .png:
                    let r = try await model.backend.decodeImage(model.doc, id: id)
                    guard let img = rgbaImage(width: Int(r.info.width), height: Int(r.info.height), data: r.data)
                    else { throw RasterError(message: r.info.error ?? "cannot make an image") }
                    let rep = NSBitmapImageRep(cgImage: img)
                    try rep.representation(using: .png, properties: [:])?.write(to: url)
                case .stored:
                    try await streamAll().write(to: url)
                }
                error = nil
            } catch let e as RasterError {
                error = e.message
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    /// All of the stream's decoded data, in protocol-sized chunks.
    private func streamAll() async throws -> Data {
        var out = Data()
        while true {
            let r = try await model.backend.streamData(
                model.doc, id: id, decoded: true, offset: UInt64(out.count), len: 4 << 20)
            if !r.info.errors.isEmpty { throw RasterError(message: r.info.errors.joined(separator: "; ")) }
            out.append(r.data)
            if r.info.eof || r.data.isEmpty { return out }
        }
    }
}

struct RasterError: Error {
    let message: String
}

/// Shows transparency behind images.
struct Checkerboard: View {
    var body: some View {
        let _ = Perf.tick("Checkerboard")
        Canvas { ctx, size in
            let s = 8.0
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
            for y in stride(from: 0.0, to: size.height, by: s) {
                for x in stride(from: 0.0, to: size.width, by: s) where (Int(x / s) + Int(y / s)) % 2 == 0 {
                    ctx.fill(Path(CGRect(x: x, y: y, width: s, height: s)), with: .color(Color(white: 0.8)))
                }
            }
        }
    }
}
