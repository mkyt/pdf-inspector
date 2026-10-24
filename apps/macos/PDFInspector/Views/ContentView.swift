import AppKit
import PdfInspectorKit
import SwiftUI

/// Operator groups (`OpView.category`), as the web UI colors and names them.
enum Category {
    static func color(_ c: String) -> Color {
        switch c {
        case "generalState": .purple
        case "specialState", "unknown": .red
        case "pathConstruction": .blue
        case "pathPainting": .indigo
        case "clipping": .cyan
        case "textObject", "textShowing": .orange
        case "textState": .yellow
        case "textPositioning": .green
        case "type3", "shading": .pink
        case "color": .teal
        case "inlineImage", "xObject": .purple
        default: .gray
        }
    }

    static func label(_ c: String) -> String {
        [
            "generalState": "general graphics state", "specialState": "q / Q / cm",
            "pathConstruction": "path construction", "pathPainting": "path painting", "clipping": "clipping",
            "textObject": "BT / ET", "textState": "text state", "textPositioning": "text positioning",
            "textShowing": "text showing", "type3": "Type 3", "color": "color", "shading": "shading",
            "inlineImage": "inline image", "xObject": "XObject", "markedContent": "marked content",
            "compatibility": "BX / EX", "unknown": "unknown",
        ][c] ?? c
    }
}

func contentTitle(_ c: ContentRef, _ s: ContentSummary?) -> String {
    switch c {
    case .page(let i): return "Page \(i + 1) — content"
    case .stream(let id):
        let kind =
            switch s?.kind {
            case .page: "page"
            case .form: "form XObject"
            case .pattern: "tiling pattern"
            default: "stream"
            }
        return "\(id.objText) — \(kind) content"
    }
}

/// The operations of a content (a page's /Contents or a stream) with their
/// problems, what each paints, and the graphics state around the selected
/// one (the web `ContentView`).
struct ContentView: View {
    let model: DocumentModel
    let content: ContentRef
    let op: UInt32?
    /// The text editor's range of operations, when it is open.
    @State private var text: Range<UInt32>?

    private func select(_ i: UInt32?) { model.nav.update(.content(content, op: i)) }

    var body: some View {
        Loaded(model.generation, fetch: { try await model.backend.content(model.doc, content) }) { s in
            VStack(spacing: 0) {
                header(s)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Divider()
                SplitPane(.horizontal, key: "content.state", size: 300, min: 200, max: 1200) {
                    SplitPane(.horizontal, key: "content.preview", size: 400, min: 200, max: 2000) {
                        if let r = text {
                            ContentTextEditor(model: model, content: content, s: s, range: r) { text = nil }
                        } else {
                            OpEditor(model: model, content: content, s: s, op: op, onSelect: select) { text = $0 }
                        }
                    } second: {
                        ContentPreview(model: model, content: content, s: s, op: op) { select($0) }
                    }
                } second: {
                    if let op, UInt64(op) < s.ops {
                        OpStatePanel(model: model, content: content, summary: s, index: op).id(op)
                    } else {
                        Text("Select an operation to see the graphics state it runs in.")
                            .font(.caption).foregroundStyle(.secondary).padding(10)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onChange(of: content) { text = nil }
    }

    private func header(_ s: ContentSummary) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if case .page(let i) = content {
                    Button("Previous Page", systemImage: "chevron.left") {
                        model.nav.open(.content(.page(index: i - 1), op: nil))
                    }
                    .disabled(i == 0)
                }
                Text(contentTitle(content, s)).font(.title3.bold())
                if case .page(let i) = content {
                    Button("Next Page", systemImage: "chevron.right") {
                        model.nav.open(.content(.page(index: i + 1), op: nil))
                    }
                    .disabled(i + 1 >= model.summary.pages)
                    Button("Page Details") { model.nav.open(.page(i)) }.labelStyle(.titleOnly)
                } else if case .stream(let id) = content {
                    Button(id.objText) { model.nav.open(.object(id)) }.labelStyle(.titleOnly)
                }
            }
            .labelStyle(.iconOnly)
            .controlSize(.small)
            HStack(spacing: 6) {
                Badge("\(s.ops.formatted()) operations")
                Badge("\(s.elements.formatted()) painting", color: .blue)
                if s.errors > 0 { Badge("\(s.errors.formatted()) errors", color: .red) }
                if s.warnings > 0 { Badge("\(s.warnings.formatted()) warnings", color: .orange) }
                if s.truncated { Badge("truncated", color: .red) }
                if s.approximate {
                    Badge("approximate text boxes", outline: true)
                        .help(
                            "Some fonts lack widths for the codes shown (standard 14 fonts outside ASCII, predefined CJK encodings): their boxes are estimates."
                        )
                }
                Text("Resources:").font(.caption).foregroundStyle(.secondary)
                if let r = s.resources {
                    ObjLink(model: model, id: r)
                } else {
                    Text(s.hasResources ? "direct" : "none").font(.caption)
                        .foregroundStyle(s.hasResources ? Color.secondary : .orange)
                }
            }
            HStack(spacing: 10) {
                ForEach(Array(s.sources.enumerated()), id: \.offset) { i, src in
                    HStack(spacing: 4) {
                        if s.sources.count > 1 { Text("[\(i)]").foregroundStyle(.secondary) }
                        if let id = src.id { ObjLink(model: model, id: id) } else { Text("direct") }
                        Text(src.error ?? "\(bytesText(src.len)) decoded")
                            .foregroundStyle(src.error != nil ? .red : .secondary)
                    }
                }
            }
            .font(.caption)
            FlowLayout(spacing: 4) {
                ForEach(s.counts, id: \.category) { c in
                    Label {
                        Text("\(Category.label(c.category)) \(c.count.formatted())")
                    } icon: {
                        Circle().fill(Category.color(c.category)).frame(width: 6, height: 6)
                    }
                    .font(.caption2)
                    .padding(.horizontal, 5)
                    .background(.quaternary.opacity(0.5), in: .capsule)
                }
            }
            if s.issues > 0 {
                ContentIssues(model: model, content: content, s: s) { select($0) }
            }
        }
    }
}

/// Lays its children out in rows, wrapping.
struct FlowLayout: Layout {
    var spacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0
        var y: CGFloat = 0
        var row: CGFloat = 0
        var widest: CGFloat = 0
        for s in subviews {
            let size = s.sizeThatFits(.unspecified)
            if x > 0 && x + size.width > width {
                y += row + spacing
                x = 0
                row = 0
            }
            x += size.width + spacing
            widest = max(widest, x - spacing)
            row = max(row, size.height)
        }
        return CGSize(width: min(widest, width), height: y + row)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        var y = bounds.minY
        var row: CGFloat = 0
        for s in subviews {
            let size = s.sizeThatFits(.unspecified)
            if x > bounds.minX && x + size.width > bounds.maxX {
                y += row + spacing
                x = bounds.minX
                row = 0
            }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            row = max(row, size.height)
        }
    }
}

private struct ContentIssues: View {
    let model: DocumentModel
    let content: ContentRef
    let s: ContentSummary
    let onPick: (UInt32) -> Void
    @State private var open: Bool?

    var body: some View {
        let open = open ?? (s.errors + s.warnings > 0)
        VStack(alignment: .leading, spacing: 2) {
            Button {
                self.open = !open
            } label: {
                Label("\(s.issues.formatted()) problem\(s.issues == 1 ? "" : "s")",
                      systemImage: open ? "chevron.down" : "chevron.right")
            }
            .buttonStyle(.link)
            if open {
                Loaded(model.generation, fetch: {
                    try await model.backend.contentIssues(model.doc, content, range: ListRange(offset: 0, limit: 500))
                }) { p in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(Array(p.items.enumerated()), id: \.offset) { _, i in
                                HStack(spacing: 6) {
                                    Badge(i.code, color: i.severity.color)
                                    Text(i.op.map { "#\($0)" } ?? "—").monospaced().foregroundStyle(.secondary)
                                        .frame(width: 56, alignment: .leading)
                                    Text(i.message).lineLimit(1).help(i.message)
                                    Spacer(minLength: 0)
                                }
                                .contentShape(.rect)
                                .onTapGesture { if let o = i.op { onPick(o) } }
                            }
                            if p.total > UInt64(p.items.count) {
                                Text("… and \((p.total - UInt64(p.items.count)).formatted()) more (see the operations).")
                                    .foregroundStyle(.secondary)
                            }
                            if s.issuesDropped > 0 {
                                Text("\(s.issuesDropped.formatted()) more were not kept.").foregroundStyle(.secondary)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 160)
                }
            }
        }
        .font(.caption)
    }
}

// MARK: The operation list

private func rangeLabel(_ from: UInt32, _ to: UInt32) -> String {
    to - from == 1 ? "#\(from)" : "#\(from)–#\(to - 1)"
}

/// The operations of a content as an editable list (the web `OpEditor`):
/// blocks fold, ⇧/⌘-click selects a range, and the keyboard edits them
/// (Return edits, I inserts, Delete removes, ⌥↑/↓ moves, copy and paste as
/// text). Edits are written as content stream text in an edit bar.
private struct OpEditor: View {
    let model: DocumentModel
    let content: ContentRef
    let s: ContentSummary
    let op: UInt32?
    let onSelect: (UInt32?) -> Void
    /// Open the text editor on operations `from..<to`.
    let onText: (Range<UInt32>) -> Void
    @Environment(\.inspectorDocument) private var document
    @State private var folds = Folds()
    @State private var selection = Set<Int>()
    @State private var bar: Bar?
    @State private var busy = false
    @State private var error: String?
    @State private var list: PagedList<OpViewPage>

    private struct Bar: Identifiable {
        let id = UUID()
        /// Replace `from..<to` (insert when equal).
        let from: UInt32
        let to: UInt32
        let title: String
        var text: String
    }

    init(
        model: DocumentModel, content: ContentRef, s: ContentSummary, op: UInt32?,
        onSelect: @escaping (UInt32?) -> Void, onText: @escaping (Range<UInt32>) -> Void
    ) {
        self.model = model
        self.content = content
        self.s = s
        self.op = op
        self.onSelect = onSelect
        self.onText = onText
        let backend = model.backend
        let doc = model.doc
        _list = State(
            initialValue: PagedList(total: Int(s.ops)) { try await backend.contentOps(doc, content, range: $0) })
        _selection = State(initialValue: op.map { [Int($0)] } ?? [])
    }

    private var total: UInt32 { UInt32(s.ops) }
    private var layout: OpLayout { OpLayout(folds: folds, total: Int(total)) }
    /// The selection as a range of operations, a folded block counting whole.
    private var range: Range<UInt32>? {
        guard let lo = selection.min(), let hiOp = selection.max() else { return nil }
        let hi = folds[UInt32(hiOp)] ?? UInt32(hiOp)
        return UInt32(lo)..<(hi + 1)
    }

    var body: some View {
        VStack(spacing: 0) {
            tools
            Divider()
            if let error {
                HStack(alignment: .top) {
                    Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
                    Spacer()
                    Button("Dismiss", systemImage: "xmark") { self.error = nil }
                        .labelStyle(.iconOnly).buttonStyle(.borderless)
                }
                .padding(6)
            }
            if let b = bar { editBar(b) }
            if total == 0 {
                Text("No operations: the content is empty. Insert some (I) to start it.")
                    .font(.callout).foregroundStyle(.secondary).padding(10)
                Spacer()
            } else {
                rows
            }
        }
        .onChange(of: model.generation) { list.reset(total: Int(s.ops)) }
        .onChange(of: s.ops) { list.reset(total: Int(s.ops)) }
        .onChange(of: op) { _, o in
            // Selected from elsewhere (rendering, problems): a single operation.
            if let o, !selection.contains(Int(o)) { selection = [Int(o)] }
        }
    }

    private var rows: some View {
        let l = layout
        return ScrollViewReader { proxy in
            List(selection: $selection) {
                ForEach(0..<l.rows, id: \.self) { r in
                    let i = l.op(r)
                    Group {
                        if let o = list.item(i) {
                            OpRow(o: o, active: op == UInt32(i), folded: folds[UInt32(i)]) {
                                toggle(UInt32(i), o.end)
                            }
                        } else {
                            ProgressView().controlSize(.mini)
                        }
                    }
                    .tag(i)
                    .id(i)
                    .onAppear { list.need(i) }
                }
            }
            .listStyle(.plain)
            .environment(\.defaultMinListRowHeight, 20)
            .contextMenu(forSelectionType: Int.self) { _ in
                menuItems
            } primaryAction: { _ in
                Task { await openEdit() }
            }
            .onChange(of: selection) { old, new in
                // The focus: the operation just added, else the first.
                let added = new.subtracting(old)
                let focus = added.max() ?? new.min()
                if let focus, UInt32(focus) != op { onSelect(UInt32(focus)) }
            }
            .onChange(of: op) { _, o in
                if let o { withAnimation { proxy.scrollTo(l.op(l.row(Int(o))), anchor: nil) } }
            }
            .onAppear {
                if let op { proxy.scrollTo(l.op(l.row(Int(op))), anchor: .center) }
            }
            .onKeyPress(.return) {
                Task { await openEdit() }
                return .handled
            }
            .onKeyPress(characters: ["i", "I", "t", "T"], phases: .down) { k in
                guard !k.modifiers.contains(.command), bar == nil else { return .ignored }
                if k.characters.lowercased() == "i" {
                    openInsert(before: k.modifiers.contains(.shift))
                } else if let r = range {
                    onText(r)
                }
                return .handled
            }
            .onKeyPress(keys: [.upArrow, .downArrow], phases: .down) { k in
                guard k.modifiers.contains(.option) else { return .ignored }
                move(down: k.key == .downArrow)
                return .handled
            }
            .onKeyPress(keys: [.leftArrow, .rightArrow], phases: .down) { k in
                guard let op, let o = list.item(Int(op)) else { return .ignored }
                let end = folds[op] ?? o.end
                guard let end, end > op, (folds[op] != nil) == (k.key == .rightArrow) else { return .ignored }
                toggle(op, end)
                return .handled
            }
            .onDeleteCommand { remove() }
            .onCopyCommand { copyProviders() }
            .onCutCommand {
                let p = copyProviders()
                remove(label: "Cut")
                return p
            }
            .onPasteCommand(of: [.utf8PlainText]) { providers in paste(providers) }
        }
    }

    @ViewBuilder private var menuItems: some View {
        Button("Edit as Text") { Task { await openEdit() } }
        Button("Insert Before…") { openInsert(before: true) }
        Button("Insert After…") { openInsert(before: false) }
        Button("Duplicate") { Task { await duplicate() } }
        Button("Wrap in q … Q") { wrap() }
        Divider()
        Button("Delete", role: .destructive) { remove() }
    }

    private var tools: some View {
        ScrollView(.horizontal) { toolButtons }
            .scrollIndicators(.never)
            .scrollBounceBehavior(.basedOnSize)
    }

    private var toolButtons: some View {
        HStack(spacing: 4) {
            Button("+ Insert") { openInsert(before: false) }.help("Insert after the selection (I)")
            Button("Edit") { Task { await openEdit() } }.help("Edit the selection as text (Return)").disabled(range == nil)
            Button("Delete") { remove() }.help("Delete the selection (Delete)").disabled(range == nil)
            Button("↑") { move(down: false) }.help("Move up (⌥↑)").disabled(range == nil || range?.lowerBound == 0)
            Button("↓") { move(down: true) }.help("Move down (⌥↓)").disabled(range == nil || (range?.upperBound ?? total) >= total)
            Button("q … Q") { wrap() }.help("Wrap in q … Q (save and restore the graphics state around it)")
                .disabled(range == nil)
            Button("Duplicate") { Task { await duplicate() } }.help("Duplicate (⌘D)")
                .keyboardShortcut("d", modifiers: .command)
                .disabled(range == nil)
            Button("Text…") {
                if let r = range, r.count > 1 { onText(r) } else { onText(0..<total) }
            }
            .help("Open the selection (or everything) in the text editor (T)")
            Spacer()
            Button("Fold All") { Task { await foldAll() } }.help("Fold every top-level block").disabled(total == 0)
            Button("Unfold") { folds = [:] }.help("Unfold every block").disabled(folds.isEmpty)
            Text(range.map { "\($0.count.formatted()) selected" } ?? "nothing selected").foregroundStyle(.secondary)
            if busy { ProgressView().controlSize(.mini) }
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .font(.caption)
        .disabled(busy)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
    }

    private func editBar(_ b: Bar) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(b.title).fontWeight(.semibold)
                Text("Content stream syntax. ⌘↩ applies, Esc cancels.").foregroundStyle(.secondary)
            }
            .font(.caption)
            HStack(alignment: .top) {
                CodeText(editing: Binding { bar?.text ?? "" } set: { bar?.text = $0 })
                    .frame(minHeight: 44, idealHeight: 70, maxHeight: 200)
                    .border(.separator)
                VStack {
                    Button("Apply") {
                        Task {
                            if await replace(b.title, b.from, b.to, bar?.text ?? "") { bar = nil }
                        }
                    }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(busy || (b.from == b.to && (bar?.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                    Button("Cancel") { bar = nil }.keyboardShortcut(.cancelAction)
                }
                .controlSize(.small)
            }
        }
        .padding(6)
        .background(.quaternary.opacity(0.4))
    }

    // MARK: Edits

    private func toggle(_ i: UInt32, _ end: UInt32?) {
        if folds[i] != nil {
            folds[i] = nil
        } else if let end, end > i {
            folds[i] = end
        }
    }

    private func foldAll() async {
        guard let r = try? await model.backend.contentBlocks(model.doc, content, depth: 0) else { return }
        folds = Dictionary(r.blocks.filter { $0.last > $0.opener }.map { ($0.opener, $0.last) }) { a, _ in a }
    }

    @discardableResult
    private func run(_ title: String, _ edits: [OpEdit], select: UInt32, folds after: Folds) async -> Bool {
        guard let document else { return false }
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await document.perform(title, [.editOps(content: content, edits: edits)])
            folds = after
            selection = [Int(select)]
            onSelect(select)
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }

    /// Replace `from..<to` with the operations in `text`.
    private func replace(_ title: String, _ from: UInt32, _ to: UInt32, _ text: String) async -> Bool {
        // How many operations the text holds is known after the edit.
        let before = s.ops
        guard
            await run(title, [.replace(from: from, to: to, text: text)], select: from, folds: folds)
        else { return false }
        if let now = try? await model.backend.content(model.doc, content) {
            let count = UInt32(max(0, Int64(now.ops) - Int64(before) + Int64(to - from)))
            folds = remapFolds(folds, from: from, to: to, count: count)
            let sel = min(from, UInt32(max(0, Int64(now.ops) - 1)))
            selection = [Int(sel)]
            onSelect(now.ops == 0 ? nil : sel)
        }
        return true
    }

    private func textOf(_ r: Range<UInt32>) async throws -> String {
        let t = try await model.backend.contentText(
            model.doc, content, range: ListRange(offset: UInt64(r.lowerBound), limit: UInt32(r.count)))
        if t.to < r.upperBound {
            throw RasterError(
                message: "the operations are too long to edit here (\(rangeLabel(r.lowerBound, r.upperBound))); use the text editor")
        }
        return t.text
    }

    private func openEdit() async {
        guard let r = range else { return }
        do {
            error = nil
            bar = Bar(from: r.lowerBound, to: r.upperBound, title: "Edit \(rangeLabel(r.lowerBound, r.upperBound))",
                      text: try await textOf(r))
        } catch let e as RasterError {
            error = e.message
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func openInsert(before: Bool) {
        let at = range.map { before ? $0.lowerBound : $0.upperBound } ?? total
        error = nil
        bar = Bar(from: at, to: at, title: at >= total ? "Add at the end" : "Insert before #\(at)", text: "")
    }

    private func remove(label: String = "Delete") {
        guard let r = range else { return }
        Task { _ = await replace("\(label) \(rangeLabel(r.lowerBound, r.upperBound))", r.lowerBound, r.upperBound, "") }
    }

    private func move(down: Bool) {
        guard let r = range, let focus = op else { return }
        let l = layout
        let lo = r.lowerBound
        let hi = r.upperBound - 1
        if down {
            let nextRow = l.row(Int(hi)) + 1
            guard nextRow < l.rows else { return }
            let next = UInt32(l.op(nextRow))
            let end = folds[next] ?? next
            let by = end - hi
            Task {
                await run(
                    "Move \(rangeLabel(lo, hi + 1)) down", [.move(from: lo, to: hi + 1, before: end + 1)],
                    select: focus + by, folds: moveFolds(folds, from: lo, to: hi + 1, before: end + 1))
                selection = Set((Int(lo + by)...Int(hi + by)))
            }
        } else {
            let prevRow = l.row(Int(lo)) - 1
            guard prevRow >= 0 else { return }
            let prev = UInt32(l.op(prevRow))
            let by = lo - prev
            Task {
                await run(
                    "Move \(rangeLabel(lo, hi + 1)) up", [.move(from: lo, to: hi + 1, before: prev)],
                    select: focus - by, folds: moveFolds(folds, from: lo, to: hi + 1, before: prev))
                selection = Set((Int(lo - by)...Int(hi - by)))
            }
        }
    }

    private func wrap() {
        guard let r = range else { return }
        let lo = r.lowerBound
        let end = r.upperBound
        Task {
            await run(
                "Wrap \(rangeLabel(lo, end)) in q … Q",
                [.replace(from: end, to: end, text: "Q"), .replace(from: lo, to: lo, text: "q")], select: lo,
                folds: remapFolds(remapFolds(folds, from: end, to: end, count: 1), from: lo, to: lo, count: 1))
        }
    }

    private func duplicate() async {
        guard let r = range else { return }
        do {
            let text = try await textOf(r)
            _ = await replace(
                "Duplicate \(rangeLabel(r.lowerBound, r.upperBound))", r.upperBound, r.upperBound, "\n" + text)
        } catch let e as RasterError {
            error = e.message
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// The selection as text, fetched when the pasteboard asks for it.
    private func copyProviders() -> [NSItemProvider] {
        guard let r = range else { return [] }
        let backend = model.backend
        let doc = model.doc
        let content = content
        let p = NSItemProvider()
        // Asked for off the main thread: a blocking request is fine there.
        p.registerDataRepresentation(for: .utf8PlainText) { done in
            do {
                let reply = try backend.sendNow(
                    .contentText(
                        doc: doc, content: content,
                        range: ListRange(offset: UInt64(r.lowerBound), limit: UInt32(r.count))))
                guard case .opText(let t) = reply.body else { throw UnexpectedResponse(body: reply.body) }
                done(Data(t.text.utf8), nil)
            } catch {
                done(nil, error)
            }
            return nil
        }
        return [p]
    }

    private func paste(_ providers: [NSItemProvider]) {
        guard let p = providers.first else { return }
        let at = range?.upperBound ?? total
        _ = p.loadDataRepresentation(for: .utf8PlainText) { data, _ in
            guard let data, let text = String(data: data, encoding: .utf8), !text.isEmpty else { return }
            Task { @MainActor in _ = await replace("Paste before #\(at)", at, at, text) }
        }
    }
}

private struct OpRow: View {
    let o: OpView
    let active: Bool
    /// Last operation of the folded block it opens.
    let folded: UInt32?
    let onFold: () -> Void

    var body: some View {
        let foldable = (o.end ?? 0) > o.index
        HStack(spacing: 4) {
            Text("\(o.index)").monospaced().foregroundStyle(.secondary).frame(width: 52, alignment: .trailing)
            Color.clear.frame(width: CGFloat(min(o.depth, 24)) * 12, height: 1)
            Image(systemName: folded != nil ? "chevron.right" : "chevron.down")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .frame(width: 12)
                .opacity(foldable ? 1 : 0)
                .contentShape(.rect)
                .onTapGesture { if foldable { onFold() } }
            Text(o.operator).monospaced().fontWeight(.semibold).foregroundStyle(Category.color(o.category))
                .frame(minWidth: 28, alignment: .leading)
            Text(o.operands.joined(separator: " ")).monospaced().lineLimit(1).layoutPriority(-1)
            if let f = folded {
                Badge("… \((f - o.index).formatted()) operations to #\(f)")
                    .onTapGesture(perform: onFold)
            }
            if let t = o.text {
                Text("“\(t)”").foregroundStyle(.orange).lineLimit(1).layoutPriority(-1)
            }
            Spacer(minLength: 4)
            if let e = o.element {
                Badge(e.kind + (e.hidden ? " (hidden)" : ""), outline: e.hidden)
            }
            if let w = o.issues.map(\.severity).max(by: { $0.rank < $1.rank }) {
                Text((o.issues.count > 1 ? "\(o.issues.count) " : "") + "\(w)")
                    .font(.caption2.bold())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 4)
                    .background(w.color, in: .capsule)
                    .help(o.issues.map { "\($0.code): \($0.message)" }.joined(separator: "\n"))
            }
        }
        .font(.caption)
        .overlay(alignment: .leading) {
            if active { Rectangle().fill(Color.accentColor).frame(width: 2) }
        }
    }
}

/// Operations `range` of a content as text, edited as a whole: applying
/// replaces them, and the backend keeps the bytes of those left alone.
private struct ContentTextEditor: View {
    let model: DocumentModel
    let content: ContentRef
    let s: ContentSummary
    let range: Range<UInt32>
    let onClose: () -> Void
    @Environment(\.inspectorDocument) private var document
    @State private var loaded: OpText?
    @State private var text = ""
    @State private var busy = false
    @State private var error: String?
    @State private var confirmClose = false

    private var dirty: Bool { loaded.map { $0.text != text } ?? false }

    var body: some View {
        let end = loaded?.to ?? range.upperBound
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text(range.lowerBound == 0 && end >= UInt32(s.ops) ? "All operations" : "Operations #\(range.lowerBound)–#\(end - 1)")
                    .fontWeight(.semibold)
                if let t = loaded, t.to < range.upperBound {
                    Text("only up to #\(t.to - 1) (the text is limited to 4 MB)").foregroundStyle(.orange)
                }
                if dirty { Text("· edited").foregroundStyle(.secondary) }
                Spacer()
                Button("Revert") { text = loaded?.text ?? "" }.disabled(!dirty || busy)
                Button("Apply") { Task { await apply(end) } }
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(!dirty || busy)
                Button("Close") { if dirty { confirmClose = true } else { onClose() } }
            }
            .font(.caption)
            .controlSize(.small)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            Divider()
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled).padding(6)
            }
            if loaded == nil && error == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                CodeText(editing: $text, wraps: false)
            }
            Text("Content stream syntax, one character per byte. Find with ⌘F.")
                .font(.caption2).foregroundStyle(.secondary).padding(4)
        }
        .task {
            do {
                let t = try await model.backend.contentText(
                    model.doc, content,
                    range: ListRange(offset: UInt64(range.lowerBound), limit: UInt32(min(UInt64(range.count), s.ops))))
                loaded = t
                text = t.text
            } catch {
                self.error = error.localizedDescription
            }
        }
        .confirmationDialog("Discard the changes to the text?", isPresented: $confirmClose) {
            Button("Discard", role: .destructive, action: onClose)
        }
    }

    private func apply(_ end: UInt32) async {
        guard let document, let t = loaded, text != t.text else { return }
        busy = true
        error = nil
        defer { busy = false }
        let whole = range.lowerBound == 0 && end == UInt32(s.ops)
        do {
            try await document.perform(
                "Edit \(whole ? "the content" : "operations #\(range.lowerBound)–#\(end - 1)") as text",
                [.editOps(content: content, edits: [.replace(from: range.lowerBound, to: end, text: text)])])
            onClose()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

// MARK: Preview

/// The rendering (or the bounding boxes) of a content, in step with the
/// selection.
private struct ContentPreview: View {
    let model: DocumentModel
    let content: ContentRef
    let s: ContentSummary
    let op: UInt32?
    let onPick: (UInt32) -> Void
    @State private var boxes = false
    @State private var step = false
    @State private var drag: Double?
    @State private var annot: UInt32?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Picker("Show", selection: $boxes) {
                    Text("Rendering").tag(false)
                    Text("Boxes").tag(true)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                Toggle("Step", isOn: $step).help("Draw only the operations up to the selected one")
            }
            .controlSize(.small)
            if boxes {
                ContentDiagram(model: model, content: content, summary: s, op: op, step: step, onPick: onPick)
            } else {
                RenderView(model: model, content: content, summary: s, op: op, step: step, onPick: onPick, annot: $annot)
                Text("Click the rendering to select the operation painting there.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if (step || boxes) && s.ops > 1 {
                Slider(
                    value: Binding { drag ?? Double(op ?? 0) } set: { drag = $0 }, in: 0...Double(s.ops - 1), step: 1
                ) { editing in
                    if !editing, let d = drag { onPick(UInt32(d)) }
                }
                .help("#\(Int(drag ?? Double(op ?? 0)))")
            }
        }
        .padding(8)
        .onChange(of: op) { drag = nil }
    }
}

private let maxElements: UInt64 = 10_000

/// What the operations paint, as bounding boxes in the content's space
/// (the web `ContentDiagram`).
private struct ContentDiagram: View {
    let model: DocumentModel
    let content: ContentRef
    let summary: ContentSummary
    let op: UInt32?
    let step: Bool
    let onPick: (UInt32) -> Void
    @State private var elements: [ElementView]?
    @State private var clips = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Toggle("Clips", isOn: $clips)
                Spacer()
                Text(
                    summary.elements > maxElements
                        ? "first \(maxElements.formatted()) of \(summary.elements.formatted())"
                        : "\(summary.elements.formatted()) boxes"
                )
                .foregroundStyle(.secondary)
            }
            .font(.caption)
            .controlSize(.small)
            if let elements {
                diagram(elements)
            } else {
                ProgressView().controlSize(.small)
            }
            Text(
                "Bounding boxes in \(summary.kind == .page ? "default user space (unrotated)" : "the stream's space"); click one to select its operation."
            )
            .font(.caption).foregroundStyle(.secondary)
        }
        .task(id: model.generation) {
            var out: [ElementView] = []
            let total = min(summary.elements, maxElements)
            var o: UInt64 = 0
            while o < total {
                guard
                    let p = try? await model.backend.contentElements(
                        model.doc, content, range: ListRange(offset: o, limit: UInt32(min(2000, total - o))))
                else { break }
                out += p.items
                if p.items.isEmpty { break }
                o += UInt64(p.items.count)
            }
            elements = out
        }
    }

    private func kindColor(_ k: String) -> Color {
        switch k {
        case "fill": .blue
        case "stroke": .teal
        case "fillStroke": .indigo
        case "text": .orange
        case "image": .purple
        case "inlineImage": .purple
        case "form": .green
        case "shading": .pink
        default: .gray
        }
    }

    private func diagram(_ all: [ElementView]) -> some View {
        let area = { (r: Rect) in (r.x1 - r.x0) * (r.y1 - r.y0) }
        let sorted = all.filter { $0.kind != "clip" || clips }.sorted { area($0.bbox) > area($1.bbox) }
        var view = summary.bounds
        if view == nil {
            for e in all {
                view = view.map {
                    Rect(x0: min($0.x0, e.bbox.x0), y0: min($0.y0, e.bbox.y0), x1: max($0.x1, e.bbox.x1), y1: max($0.y1, e.bbox.y1))
                } ?? e.bbox
            }
        }
        let b = view ?? Rect(x0: 0, y0: 0, x1: 100, y1: 100)
        let w = max(b.x1 - b.x0, 1)
        let h = max(b.y1 - b.y0, 1)
        let pad = max(w, h) * 0.03
        let selected = all.first { $0.op == op }
        return GeometryReader { geo in
            let k = min(geo.size.width / (w + 2 * pad), geo.size.height / (h + 2 * pad))
            // Content space (y up) to the canvas.
            let m = CGAffineTransform(a: k, b: 0, c: 0, d: -k, tx: (pad - b.x0) * k, ty: (b.y1 + pad) * k)
            let rect = { (r: Rect) in
                Path(CGRect(x: r.x0, y: r.y0, width: max(r.x1 - r.x0, 0), height: max(r.y1 - r.y0, 0))).applying(m)
            }
            Canvas { ctx, _ in
                ctx.fill(rect(b), with: .color(Color(nsColor: .textBackgroundColor)))
                ctx.stroke(rect(b), with: .color(.gray.opacity(0.5)), lineWidth: 1)
                for e in sorted {
                    let later = step && op != nil && e.op > op!
                    let c = kindColor(e.kind)
                    let p = rect(e.bbox)
                    if e.kind != "clip" { ctx.fill(p, with: .color(c.opacity(later ? 0.02 : 0.1))) }
                    ctx.stroke(
                        p, with: .color(c.opacity(later ? 0.15 : 0.8)),
                        style: StrokeStyle(lineWidth: 1, dash: e.hidden || e.kind == "clip" ? [3, 2] : []))
                }
                if let s = selected {
                    ctx.fill(rect(s.bbox), with: .color(.red.opacity(0.15)))
                    ctx.stroke(rect(s.bbox), with: .color(.red), lineWidth: 2.5)
                }
            }
            .background(Color.gray.opacity(0.12))
            .contentShape(.rect)
            .onTapGesture(coordinateSpace: .local) { p in
                let q = p.applying(m.inverted())
                // The smallest box under the click.
                if let e = sorted.last(where: {
                    q.x >= $0.bbox.x0 && q.x <= $0.bbox.x1 && q.y >= $0.bbox.y0 && q.y <= $0.bbox.y1
                }) {
                    onPick(e.op)
                }
            }
            .frame(width: (w + 2 * pad) * k, height: (h + 2 * pad) * k)
        }
        .aspectRatio((w + 2 * pad) / (h + 2 * pad), contentMode: .fit)
        .frame(maxHeight: 520)
    }
}
