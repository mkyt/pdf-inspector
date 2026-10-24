import PdfInspectorKit
import SwiftUI

/// The file's bytes (the web `HexView`). Only the rows in view are fetched
/// (64 KiB chunks) and the scroll range is scaled, so files of any size fit.
/// Follows the object the window shows; other views highlight spans in it.
struct HexView: View {
    let model: DocumentModel
    @State private var top = 0
    @State private var viewHeight = 200.0
    @State private var chunks: [UInt64: Data] = [:]
    @State private var position = ScrollPosition(y: 0)
    @State private var goto = ""

    private static let row = 16
    private static let lineHeight = 15.0
    private static let chunk: UInt64 = 64 * 1024
    /// Keep the content height reasonable; scroll positions map to rows.
    private static let maxScroll = 4_000_000.0

    private var size: UInt64 { model.summary.info.size }
    private var totalRows: Int { max(1, Int((size + UInt64(Self.row) - 1) / UInt64(Self.row))) }
    private var visible: Int { max(1, Int((viewHeight - 4) / Self.lineHeight)) }
    private var maxTop: Int { max(0, totalRows - visible) }
    private var spacer: Double { min(Double(totalRows) * Self.lineHeight, Self.maxScroll) }

    private struct ChunkKey: Equatable {
        let first: UInt64
        let last: UInt64
        let generation: Int
    }

    var body: some View {
        let first = UInt64(top * Self.row)
        let last = min(size, UInt64((top + visible) * Self.row))
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Text("File bytes").font(.caption.weight(.semibold))
                TextField("Go to offset", text: $goto, prompt: Text("Go to offset (dec or 0x…)"))
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.small)
                    .frame(width: 180)
                    .onSubmit(jump)
                if let s = model.nav.hexSpan {
                    Text("selection \(s.text)").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            Divider()
            ScrollView {
                Color.clear.frame(height: spacer)
            }
            .scrollPosition($position)
            .onScrollGeometryChange(for: Double.self, of: { $0.contentOffset.y }) { _, y in
                let range = max(1, spacer - viewHeight)
                top = min(maxTop, max(0, Int((y / range * Double(maxTop)).rounded())))
            }
            .onGeometryChange(for: Double.self, of: { $0.size.height }) { viewHeight = $0 }
            .overlay(alignment: .topLeading) {
                rows.allowsHitTesting(false)
            }
            .clipped()
        }
        .task(id: ChunkKey(first: first / Self.chunk, last: max(last, first + 1) / Self.chunk,
                           generation: model.generation)) {
            await load(from: first, to: max(last, first + 1))
        }
        .onChange(of: model.generation) { chunks = [:] }
        .onChange(of: model.nav.hexSpan) { _, s in
            if let s { scroll(to: Int(s.start / UInt64(Self.row)) - 2) }
        }
        .task(id: FollowKey(target: model.nav.target, generation: model.generation)) {
            await follow()
        }
    }

    private var rows: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(top..<min(totalRows, top + visible), id: \.self) { r in
                Text(line(r))
                    .font(.system(size: 11, design: .monospaced))
                    .frame(height: Self.lineHeight)
            }
        }
        .padding(.leading, 8)
        .padding(.top, 2)
    }

    private func byte(_ off: UInt64) -> UInt8? {
        guard let c = chunks[off / Self.chunk] else { return nil }
        let i = Int(off % Self.chunk)
        return i < c.count ? c[c.startIndex + i] : nil
    }

    private func line(_ r: Int) -> AttributedString {
        let base = UInt64(r * Self.row)
        let off = String(base, radix: 16)
        var out = AttributedString(String(repeating: "0", count: max(0, 10 - off.count)) + off + "  ")
        out.foregroundColor = .secondary
        var ascii = ""
        let hl = model.nav.hexSpan
        for i in 0..<Self.row {
            let o = base + UInt64(i)
            guard o < size else {
                out += AttributedString("   ")
                continue
            }
            let b = byte(o)
            var h = AttributedString(b.map(hexByte) ?? "··")
            if let hl, o >= hl.start, o < hl.end {
                h.backgroundColor = .yellow.opacity(0.45)
            }
            out += h
            out += AttributedString(i < Self.row - 1 ? " " : "")
            ascii.append(b.map(asciiByte) ?? " ")
        }
        var a = AttributedString("  " + ascii)
        a.foregroundColor = .secondary
        return out + a
    }

    private func load(from first: UInt64, to last: UInt64) async {
        var c = first / Self.chunk
        while c * Self.chunk < last {
            if chunks[c] == nil {
                guard
                    let b = try? await model.backend.rawBytes(
                        model.doc, offset: c * Self.chunk, len: UInt32(Self.chunk))
                else { return }
                // Keep the chunks around the view.
                if chunks.count > 32 { chunks = chunks.filter { abs(Int($0.key) - Int(c)) < 8 } }
                chunks[c] = b.data
            }
            c += 1
        }
    }

    private func scroll(to row: Int) {
        let t = max(0, min(maxTop, row))
        position.scrollTo(y: maxTop > 0 ? Double(t) / Double(maxTop) * max(0, spacer - viewHeight) : 0)
        top = t
    }

    private func jump() {
        let s = goto.trimmingCharacters(in: .whitespaces).lowercased()
        let n = s.hasPrefix("0x") ? UInt64(s.dropFirst(2), radix: 16) : UInt64(s)
        if let n { model.nav.showBytes(ByteSpan(start: n, end: n + 1)) }
    }

    private struct FollowKey: Equatable {
        let target: Target
        let generation: Int

        init(target: Target, generation: Int) {
            // The selected operation is not another object.
            if case .content(let c, _) = target {
                self.target = .content(c, op: nil)
            } else {
                self.target = target
            }
            self.generation = generation
        }
    }

    /// Select the bytes of the object shown ("N G obj … endobj"); for an
    /// object inside an object stream, those of the stream.
    private func follow() async {
        guard let id = try? await targetObject(model) else { return }
        guard var o = try? await model.backend.object(model.doc, id: id) else { return }
        if case .objStm(let stream, _) = o.location {
            guard let s = try? await model.backend.object(model.doc, id: stream) else { return }
            o = s
        }
        if case .file(let span) = o.location { model.nav.showBytes(span) }
    }
}

/// The object the window's target is about, if any.
@MainActor
func targetObject(_ model: DocumentModel) async throws -> ObjRef? {
    switch model.nav.target {
    case .object(let id), .font(let id), .content(.stream(let id), _): id
    case .page(let i), .content(.page(let i), _): try await model.backend.page(model.doc, index: i).id
    case .document: model.summary.root
    case .trailer: nil
    }
}
