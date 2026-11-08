import AppKit
import PdfInspectorKit
import SwiftUI

/// The script at `path`: a JavaScript action, or its /JS string or stream
/// (the web `ScriptView`).
struct ScriptView: View {
    let model: DocumentModel
    let path: NodePath
    @State private var script: ScriptText?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let s = script {
                HStack(spacing: 8) {
                    Text("\(s.text.utf16.count.formatted()) characters · \(s.stream == nil ? "a string" : "in a stream")")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    // On the stream itself, a link to it would lead nowhere.
                    if let id = s.stream, path != NodePath(root: .object(id: id), path: []) {
                        LinkButton("\(id.objText) →") { model.nav.open(.object(id)) }.font(.caption)
                    }
                }
                if s.truncated {
                    Text("Only the beginning of the script is shown.").font(.caption).foregroundStyle(.blue)
                }
                ForEach(s.errors, id: \.self) { e in
                    Text(e).font(.caption).foregroundStyle(.red)
                }
                ScriptCode(text: s.text)
            } else if let error {
                Text(error).font(.caption).foregroundStyle(.red)
                Spacer()
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(8)
        .task(id: model.generation) {
            do {
                script = try await model.backend.script(model.doc, path: path)
                error = nil
            } catch is CancellationError {
            } catch {
                self.error = error.localizedDescription
                script = nil
            }
        }
    }
}

/// Read-only JavaScript in an NSTextView, highlighted.
private struct ScriptCode: NSViewRepresentable {
    let text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        let tv = scroll.documentView as! NSTextView
        tv.isRichText = false
        tv.isEditable = false
        tv.isSelectable = true
        tv.usesFindBar = true
        tv.isIncrementalSearchingEnabled = true
        tv.textContainerInset = NSSize(width: 4, height: 4)
        show(text, in: tv)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let tv = scroll.documentView as! NSTextView
        if tv.string != text { show(text, in: tv) }
    }

    private func show(_ text: String, in tv: NSTextView) {
        let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        let s = NSMutableAttributedString(
            string: text, attributes: [.font: font, .foregroundColor: NSColor.labelColor])
        let bold = NSFont.monospacedSystemFont(ofSize: 11, weight: .semibold)
        let italic = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        for (range, token) in highlightJavaScript(text) {
            switch token {
            case .keyword: s.addAttributes([.foregroundColor: NSColor.systemPurple, .font: bold], range: range)
            case .literal, .number: s.addAttribute(.foregroundColor, value: NSColor.systemTeal, range: range)
            case .string: s.addAttribute(.foregroundColor, value: NSColor.systemOrange, range: range)
            case .regexp: s.addAttribute(.foregroundColor, value: NSColor.systemRed, range: range)
            case .comment:
                s.addAttributes([.foregroundColor: NSColor.secondaryLabelColor, .font: italic], range: range)
            case .function: s.addAttribute(.foregroundColor, value: NSColor.systemBlue, range: range)
            case .property: break
            }
        }
        tv.textStorage?.setAttributedString(s)
    }
}
