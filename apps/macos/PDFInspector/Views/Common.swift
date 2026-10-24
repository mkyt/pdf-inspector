import AppKit
import PdfInspectorKit
import SwiftUI

/// Something read from the backend, read again when `key` changes; what was
/// read before stays shown meanwhile.
struct Loaded<Key: Equatable, Value, Content: View>: View {
    let key: Key
    let fetch: () async throws -> Value
    @ViewBuilder let content: (Value) -> Content
    @State private var value: Value?
    @State private var error: String?

    init(
        _ key: Key, fetch: @escaping () async throws -> Value,
        @ViewBuilder content: @escaping (Value) -> Content
    ) {
        self.key = key
        self.fetch = fetch
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let value {
                content(value)
            } else if let error {
                Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .task(id: key) {
            do {
                value = try await fetch()
                error = nil
            } catch is CancellationError {
            } catch {
                self.error = error.localizedDescription
                value = nil
            }
        }
    }
}

/// Monospaced text in an NSTextView: read-only for data of any size, or
/// editable (no smart quotes or other substitutions).
struct CodeText: NSViewRepresentable {
    @Binding var text: String
    var editable = false
    var wraps = true

    init(_ text: String, wraps: Bool = true) {
        _text = .constant(text)
        self.wraps = wraps
    }

    init(editing text: Binding<String>, wraps: Bool = true) {
        _text = text
        editable = true
        self.wraps = wraps
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        let tv = scroll.documentView as! NSTextView
        tv.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        tv.isRichText = false
        tv.isEditable = editable
        tv.isSelectable = true
        tv.allowsUndo = true
        tv.usesFindBar = true
        tv.isIncrementalSearchingEnabled = true
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.isAutomaticTextReplacementEnabled = false
        tv.isAutomaticSpellingCorrectionEnabled = false
        tv.isContinuousSpellCheckingEnabled = false
        tv.textContainerInset = NSSize(width: 4, height: 4)
        tv.delegate = context.coordinator
        if !wraps {
            tv.isHorizontallyResizable = true
            tv.textContainer?.widthTracksTextView = false
            tv.textContainer?.containerSize = NSSize(
                width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
            scroll.hasHorizontalScroller = true
        }
        tv.string = text
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        let tv = scroll.documentView as! NSTextView
        if tv.string != text { tv.string = text }
        tv.isEditable = editable
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: CodeText

        init(_ parent: CodeText) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard parent.editable, let tv = notification.object as? NSTextView else { return }
            parent.text = tv.string
        }
    }
}

/// Hex dump of bytes in memory; `base` is the offset of the first one.
struct BytesDump: View {
    let data: Data
    var base: UInt64 = 0

    var body: some View {
        let rows = (data.count + 15) / 16
        List(0..<rows, id: \.self) { r in
            Text(line(r))
                .font(.system(size: 11, design: .monospaced))
                .textSelection(.enabled)
                .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
        }
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, 15)
    }

    private func line(_ r: Int) -> String {
        let start = data.startIndex + r * 16
        let row = data[start..<min(start + 16, data.endIndex)]
        let off = String(base + UInt64(r * 16), radix: 16)
        let hex = row.map(hexByte).joined(separator: " ")
        return String(repeating: "0", count: max(0, 8 - off.count)) + off + "  "
            + hex.padding(toLength: 47, withPad: " ", startingAt: 0) + "  "
            + String(row.map(asciiByte))
    }
}

/// A compact list of problems; clicking one shows its bytes.
struct IssueList: View {
    let model: DocumentModel
    let issues: [DiagnosticInfo]
    var max = 8
    var withObj = false
    @State private var all = false

    var body: some View {
        let sorted = issues.sorted { $0.severity.rank > $1.severity.rank }
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array((all ? sorted : Array(sorted.prefix(max))).enumerated()), id: \.offset) { _, d in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(d.code)
                        .font(.caption2.monospaced())
                        .padding(.horizontal, 4)
                        .background(d.severity.color.opacity(0.18), in: .rect(cornerRadius: 3))
                        .foregroundStyle(d.severity.color)
                    Text("\(Text(issueWhere(d)).monospaced().foregroundStyle(.secondary))\(d.message)")
                        .font(.caption)
                        .textSelection(.enabled)
                }
                .contentShape(.rect)
                .onTapGesture {
                    if let s = d.span { model.nav.showBytes(s, reveal: true) }
                }
            }
            if sorted.count > max {
                Button(all ? "Show fewer" : "Show all \(sorted.count)") { all.toggle() }
                    .buttonStyle(.link)
                    .font(.caption)
            }
        }
    }

    private func issueWhere(_ d: DiagnosticInfo) -> String {
        let obj = withObj ? (d.obj?.objText ?? "trailer") : ""
        let w = obj + (d.path ?? "")
        return w.isEmpty ? "" : w + " "
    }
}

/// A small rounded label.
struct Badge: View {
    let text: String
    var color: Color = .secondary
    var outline = false

    init(_ text: String, color: Color = .secondary, outline: Bool = false) {
        self.text = text
        self.color = color
        self.outline = outline
    }

    var body: some View {
        Text(text)
            .font(.caption)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .foregroundStyle(color)
            .background {
                if outline {
                    RoundedRectangle(cornerRadius: 4).strokeBorder(color.opacity(0.6))
                } else {
                    RoundedRectangle(cornerRadius: 4).fill(color.opacity(0.15))
                }
            }
    }
}

/// Reports about edits and saves, at the bottom of the window.
struct MessagesOverlay: View {
    let model: DocumentModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(model.messages) { m in
                HStack(alignment: .top, spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(m.title).fontWeight(.semibold)
                        ForEach(Array(m.lines.enumerated()), id: \.offset) { _, l in
                            Text(l).textSelection(.enabled)
                        }
                    }
                    Spacer(minLength: 0)
                    Button("Dismiss", systemImage: "xmark") { model.dismiss(m.id) }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.plain)
                }
                .font(.caption)
                .foregroundStyle(.white)
                .padding(8)
                .frame(width: 420, alignment: .leading)
                .background(m.kind.color, in: .rect(cornerRadius: 8))
                .shadow(radius: 4)
            }
        }
        .padding(12)
        .animation(.default, value: model.messages.map(\.id))
    }
}

extension DocumentModel.Message.Kind {
    var color: Color {
        switch self {
        case .info: .blue
        case .warning: .orange
        case .error: .red
        }
    }
}

extension InspectorDocument {
    /// Like `perform`, but errors are reported instead of thrown.
    func run(_ label: String, _ commands: [EditCommand]) {
        Task {
            do {
                try await perform(label, commands)
            } catch {
                model?.report(.error, "\(label) failed", [error.localizedDescription])
            }
        }
    }
}

/// A link-styled button.
struct LinkButton: View {
    let title: String
    let action: () -> Void

    init(_ title: String, action: @escaping () -> Void) {
        self.title = title
        self.action = action
    }

    var body: some View {
        Button(title, action: action).buttonStyle(.link)
    }
}

/// Two panes with a divider to drag: `second` below (`.vertical`) or to the
/// right (`.horizontal`) of `first`, its size kept across launches under
/// `key`. (NSSplitView-backed `VSplitView` loops in layout beside an
/// inspector.)
struct SplitPane<First: View, Second: View>: View {
    let axis: Axis
    let showSecond: Bool
    let min: Double
    let max: Double
    @AppStorage private var size: Double
    @State private var start: Double?
    @State private var length = Double.infinity
    @ViewBuilder let first: () -> First
    @ViewBuilder let second: () -> Second

    init(
        _ axis: Axis, key: String, size: Double, min: Double, max: Double, showSecond: Bool = true,
        @ViewBuilder first: @escaping () -> First, @ViewBuilder second: @escaping () -> Second
    ) {
        self.axis = axis
        self.showSecond = showSecond
        self.min = min
        self.max = max
        _size = AppStorage(wrappedValue: size, "pane.\(key)")
        self.first = first
        self.second = second
    }

    /// Room left for `first`, whatever `second` asks for.
    private static var firstMin: Double { 120 }

    var body: some View {
        let layout = axis == .vertical ? AnyLayout(VStackLayout(spacing: 0)) : AnyLayout(HStackLayout(spacing: 0))
        // The size asked for, within what the pane has.
        let s = Swift.max(40, Swift.min(Swift.min(max, Swift.max(min, size)), length - Self.firstMin))
        layout {
            first()
                .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity, alignment: .topLeading)
                .clipped()
            if showSecond {
                divider
                second()
                    .frame(width: axis == .horizontal ? s : nil, height: axis == .vertical ? s : nil)
                    .clipped()
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
        .onGeometryChange(for: Double.self, of: { axis == .vertical ? $0.size.height : $0.size.width }) {
            length = $0
        }
    }

    private var divider: some View {
        Rectangle()
            .fill(.separator)
            .frame(width: axis == .horizontal ? 1 : nil, height: axis == .vertical ? 1 : nil)
            .padding(axis == .vertical ? .vertical : .horizontal, 3)
            .contentShape(.rect)
            .padding(axis == .vertical ? .vertical : .horizontal, -3)
            .pointerStyle(axis == .vertical ? .rowResize : .columnResize)
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { g in
                        let from = start ?? size
                        start = from
                        let d = axis == .vertical ? g.translation.height : g.translation.width
                        size = Swift.min(max, Swift.max(min, from - d))
                    }
                    .onEnded { _ in start = nil }
            )
            .onTapGesture(count: 2) { size = (min + max) / 2 }
            .zIndex(1)
    }
}
