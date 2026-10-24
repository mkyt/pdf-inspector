import AppKit
import PdfInspectorKit
import SwiftUI

/// A stream's data, decoded or as stored, as text or hex; editing it and
/// its encoding (the web `StreamPanel`).
struct StreamPanel: View {
    let model: DocumentModel
    let id: ObjRef
    let info: StreamInfo
    @State private var decoded = true
    @State private var hex = false
    @State private var chunks: [Bytes] = []
    @State private var error: String?
    @State private var loading = false
    @State private var editing = false
    @Environment(\.inspectorDocument) private var document

    private static let chunk: UInt32 = 256 * 1024

    private struct Key: Equatable {
        let decoded: Bool
        let generation: Int
    }

    var body: some View {
        let data = chunks.reduce(into: Data()) { $0.append($1.data) }
        let last = chunks.last?.info
        let errors = Array(Set(chunks.flatMap(\.info.errors))).sorted()
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Picker("Data", selection: $decoded) {
                    Text("Decoded").tag(true)
                    Text("Raw").tag(false)
                }
                .fixedSize()
                Picker("Show", selection: $hex) {
                    Text("Text").tag(false)
                    Text("Hex").tag(true)
                }
                .fixedSize()
                Text(
                    "filters: \(info.filters.isEmpty ? "none" : info.filters.joined(separator: ", "))"
                        + (info.encrypted ? " · encrypted" : "") + " · stored \(bytesText(info.rawLen)) · "
                        + (last?.total.map { "\(bytesText($0)) total" } ?? "\(bytesText(UInt64(data.count))) loaded")
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                if let span = info.dataSpan {
                    LinkButton("Show in file") { model.nav.showBytes(span, reveal: true) }.font(.caption)
                }
                Spacer()
                Button("Edit Data…") { editing = true }
                Menu("Encoding") {
                    Button("Compress (FlateDecode)") {
                        document?.run("Compress the stream", [.setStreamFilter(id: id, filter: .flate)])
                    }
                    Button("Store Decoded (No Filter)") {
                        document?.run("Decompress the stream", [.setStreamFilter(id: id, filter: .none)])
                    }
                    Divider()
                    Text("Image codecs (DCT, JPX, JBIG2, CCITT) stay.")
                }
                .fixedSize()
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            if let s = last?.stoppedAt, decoded {
                Text("Data is still /\(s) encoded (image codec or unsupported filter).")
                    .font(.caption).foregroundStyle(.blue)
            }
            ForEach(errors, id: \.self) { e in
                Text(e).font(.caption).foregroundStyle(.red)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            if hex {
                BytesDump(data: data)
            } else {
                CodeText(bytesAsText(data))
            }
            if let last, !last.eof {
                Button("Load next \(bytesText(UInt64(Self.chunk)))") { Task { await more() } }
                    .disabled(loading)
                    .controlSize(.small)
            }
        }
        .padding(8)
        .task(id: Key(decoded: decoded, generation: model.generation)) {
            chunks = []
            await more()
        }
        .sheet(isPresented: $editing) {
            DataEditor(model: model, id: id, info: info)
        }
    }

    private func more() async {
        loading = true
        defer { loading = false }
        do {
            let offset = chunks.last.map { $0.info.offset + $0.info.len } ?? 0
            chunks.append(
                try await model.backend.streamData(
                    model.doc, id: id, decoded: decoded, offset: offset, len: Self.chunk))
            error = nil
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// Replace a stream's decoded data: as text (bytes as Latin-1 characters),
/// or from a file.
private struct DataEditor: View {
    let model: DocumentModel
    let id: ObjRef
    let info: StreamInfo
    @Environment(\.dismiss) private var dismiss
    @Environment(\.inspectorDocument) private var document
    @State private var text: String?
    @State private var binary = false
    @State private var stoppedAt: String?
    @State private var file: URL?
    @State private var filter: StreamFilter = .none
    @State private var error: String?
    @State private var busy = false

    /// Largest decoded data edited as text.
    private static let maxText: UInt32 = 4 << 20

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Stream data of \(id.num) \(id.generation)").font(.headline)
            if let s = stoppedAt {
                Text("The data stays /\(s) encoded: what you give is that codec’s input.")
                    .font(.caption).foregroundStyle(.blue)
            }
            if text == nil && error == nil {
                ProgressView().frame(maxWidth: .infinity, minHeight: 200)
            } else if binary {
                Text("The data is binary (or too large to edit as text): replace it with a file.")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Choose File…") { choose() }
                    Text(file?.lastPathComponent ?? "No file chosen").foregroundStyle(.secondary)
                }
                Spacer()
            } else {
                Text("Each character is one byte (Latin-1). Line breaks are stored as LF.")
                    .font(.caption).foregroundStyle(.secondary)
                CodeText(editing: Binding { text ?? "" } set: { text = $0 })
                    .frame(minHeight: 260)
                    .border(.separator)
            }
            HStack {
                Text("Store")
                Picker("Store", selection: $filter) {
                    Text("Compressed (Flate)").tag(StreamFilter.flate)
                    Text("Uncompressed").tag(StreamFilter.none)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                Spacer()
                if !binary {
                    Button("Replace with a File Instead") { binary = true }.buttonStyle(.link)
                }
            }
            .font(.caption)
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await submit() } }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(busy)
            }
        }
        .padding()
        .frame(minWidth: 640, minHeight: 420)
        .task {
            filter = info.filters.contains("FlateDecode") ? .flate : .none
            do {
                let b = try await model.backend.streamData(
                    model.doc, id: id, decoded: true, offset: 0, len: Self.maxText)
                let t = latin1(b.data)
                stoppedAt = b.info.stoppedAt
                binary = !b.info.eof || !b.info.errors.isEmpty
                    || b.data.contains { $0 < 0x20 && ![9, 10, 12, 13].contains($0) }
                text = t
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        if panel.runModal() == .OK { file = panel.url }
    }

    private func submit() async {
        guard let document else { return }
        busy = true
        error = nil
        defer { busy = false }
        let data: Data?
        if binary {
            data = file.flatMap { try? Data(contentsOf: $0) }
        } else {
            data = fromLatin1(text ?? "")
        }
        guard let data else {
            error = binary ? "Choose a file." : "Only characters up to U+00FF can be stored as bytes."
            return
        }
        do {
            try await document.perform(
                "Edit the data of \(id.num) \(id.generation)",
                [.setStreamData(id: id, data: .base64(data: data.base64EncodedString()), filter: filter)])
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
