import AppKit
import ImageIO
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
/// Codecs ImageIO reads as they are stored.
private let fileCodecs: [String: (label: String, ext: String, type: UTType)] = [
    "DCTDecode": ("JPEG", "jpg", .jpeg),
    "JPXDecode": ("JPEG 2000", "jp2", UTType("public.jpeg-2000") ?? .data),
]

/// A preview of an image XObject (the web `ImagePreview`): JPEG and
/// JPEG 2000 through ImageIO, uncompressed or Flate data with 8-bit gray /
/// RGB / CMYK samples (and 1-bit masks) drawn here. Downloads the image as
/// a file.
struct ImagePreview: View {
    let model: DocumentModel
    let id: ObjRef
    let info: StreamInfo
    @State private var dict: ImageDict?
    @State private var image: CGImage?
    @State private var note: String?
    @State private var error: String?

    private static let maxData: UInt32 = 4 << 20
    private static let maxSide = 320.0

    private var codec: String? { info.filters.last.flatMap { fileCodecs[$0] != nil ? $0 : nil } }

    private func drawable(_ d: ImageDict) -> Bool {
        info.filters.allSatisfy(plainFilters.contains)
            && (d.bits == 8 || (d.bits == 1 && (d.mask || d.space == "DeviceGray")))
    }

    var body: some View {
        let _ = Perf.tick("ImagePreview")
        if let d = dict {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text("Image").fontWeight(.semibold)
                    Text("\(d.width) × \(d.height) · \(d.mask ? "ImageMask" : d.space ?? "other color space") · \(d.bits) bpc")
                        .foregroundStyle(.secondary)
                    if let c = codec, let k = fileCodecs[c] {
                        Button("Save \(k.label)…") { save(d, as: .stored(k.ext, k.type)) }
                    } else if drawable(d) {
                        Button("Save PNG…") { save(d, as: .png) }
                    }
                    Button(samplesLabel) { save(d, as: .stored("bin", .data)) }
                }
                .buttonStyle(.link)
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
        guard codec != nil || drawable(d) else {
            note = "No preview for this encoding."
            return
        }
        do {
            let b = try await model.backend.streamData(model.doc, id: id, decoded: true, offset: 0, len: Self.maxData)
            if codec != nil {
                image = decodeFile(b.data)
                if image == nil { note = b.info.eof ? "ImageIO cannot read this image." : "Too large to preview here." }
            } else if !b.info.eof {
                note = "Too large to preview here."
            } else {
                switch rasterize(d, b.data) {
                case .success(let i): image = i
                case .failure(let e): note = e.message
                }
            }
        } catch is CancellationError {
        } catch {
            note = error.localizedDescription
        }
    }

    private enum SaveKind {
        case png
        case stored(String, UTType)
    }

    private func save(_ d: ImageDict, as kind: SaveKind) {
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
                let data = try await streamAll()
                switch kind {
                case .png:
                    let img = try rasterize(d, data).get()
                    let rep = NSBitmapImageRep(cgImage: img)
                    try rep.representation(using: .png, properties: [:])?.write(to: url)
                case .stored:
                    try data.write(to: url)
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
                model.doc, id: id, decoded: true, offset: UInt64(out.count), len: Self.maxData)
            if !r.info.errors.isEmpty { throw RasterError(message: r.info.errors.joined(separator: "; ")) }
            out.append(r.data)
            if r.info.eof || r.data.isEmpty { return out }
        }
    }
}

struct RasterError: Error {
    let message: String
}

private func decodeFile(_ data: Data) -> CGImage? {
    guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
    return CGImageSourceCreateImageAtIndex(src, 0, nil)
}

/// Samples as an RGBA image, or why they cannot be.
func rasterize(_ d: ImageDict, _ bytes: Data) -> Result<CGImage, RasterError> {
    let (w, h) = (d.width, d.height)
    var px = [UInt8](repeating: 0, count: w * h * 4)
    let b = [UInt8](bytes)
    if d.bits == 1 {
        let row = (w + 7) / 8
        guard b.count >= row * h else { return .failure(RasterError(message: "\(b.count) bytes for \(w)×\(h)")) }
        for y in 0..<h {
            for x in 0..<w {
                let bit = (b[y * row + (x >> 3)] >> (7 - UInt8(x & 7))) & 1
                let o = 4 * (y * w + x)
                // Image masks paint where the sample is 0; gray 1 is white.
                let v: UInt8 = bit == 1 ? 255 : 0
                px[o] = d.mask ? 0 : v
                px[o + 1] = px[o]
                px[o + 2] = px[o]
                px[o + 3] = d.mask ? 255 - v : 255
            }
        }
    } else {
        let n = b.count / (w * h)
        let comps =
            switch d.space {
            case "DeviceGray": 1
            case "DeviceRGB": 3
            case "DeviceCMYK": 4
            default: n
            }
        guard [1, 3, 4].contains(comps), b.count >= w * h * comps else {
            return .failure(RasterError(message: "cannot tell the samples apart (\(b.count) bytes for \(w)×\(h))"))
        }
        for i in 0..<(w * h) {
            let s = i * comps
            let o = 4 * i
            switch comps {
            case 1:
                px[o] = b[s]
                px[o + 1] = b[s]
                px[o + 2] = b[s]
            case 3:
                px[o] = b[s]
                px[o + 1] = b[s + 1]
                px[o + 2] = b[s + 2]
            default:
                let k = Int(b[s + 3])
                px[o] = UInt8(255 - min(255, Int(b[s]) + k))
                px[o + 1] = UInt8(255 - min(255, Int(b[s + 1]) + k))
                px[o + 2] = UInt8(255 - min(255, Int(b[s + 2]) + k))
            }
            px[o + 3] = 255
        }
    }
    let info = BitmapInfo(
        part: 0, rect: PixelRect(x: 0, y: 0, width: UInt32(w), height: UInt32(h)), fullWidth: UInt32(w),
        fullHeight: UInt32(h), scale: 1, ops: 0, notes: [])
    guard let img = Bitmap(info: info, data: Data(px)).cgImage() else {
        return .failure(RasterError(message: "cannot make an image"))
    }
    return .success(img)
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
