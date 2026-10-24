import PdfInspectorKit
import SwiftUI

extension GlyphSourceKind {
    var text: String {
        switch self {
        case .embedded: "embedded"
        case .substitute: "substituted"
        case .procedures: "glyph procedures"
        case .none: "no glyphs"
        }
    }

    var color: Color {
        switch self {
        case .embedded: .teal
        case .substitute: .blue
        case .procedures: .purple
        case .none: .orange
        }
    }
}

/// A font: its dictionaries, encoding, program, problems, the text it
/// shows and the glyph table (the web `FontView`).
struct FontView: View {
    let model: DocumentModel
    let id: ObjRef

    var body: some View {
        Loaded(model.generation, fetch: { try await model.backend.font(model.doc, id: id) }) { f in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 8) {
                        Text(f.baseFont ?? "(no /BaseFont)").font(.title3.bold()).textSelection(.enabled)
                        Badge((f.subtype ?? "?") + (f.cidSubtype.map { " · \($0)" } ?? ""))
                        Badge(f.glyphSource.text + (f.substitute.map { ": \($0)" } ?? ""), color: f.glyphSource.color)
                        Button(f.id.objText) { model.nav.open(.object(f.id)) }.controlSize(.small)
                    }
                    if !f.issues.isEmpty {
                        IssueList(model: model, issues: f.issues)
                            .padding(6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background((f.issues.worst ?? .info).color.opacity(0.08), in: .rect(cornerRadius: 6))
                    }
                    FontProperties(model: model, f: f)
                    FontUses(model: model, f: f)
                    GlyphTable(model: model, f: f)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

private struct FontProperties: View {
    let model: DocumentModel
    let f: FontDetail

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 3) {
            if let d = f.descendant { row("descendant") { ObjLink(model: model, id: d) } }
            row("encoding") { Text((f.encoding ?? "none (built-in)") + (f.vertical ? " · vertical" : "")) }
            if let c = f.cidToGid { row("CIDToGIDMap") { Text(c) } }
            row("ToUnicode") {
                if let t = f.toUnicode {
                    HStack {
                        ObjLink(model: model, id: t)
                        Text("\(f.toUnicodeEntries.formatted()) entries").foregroundStyle(.secondary)
                    }
                } else {
                    Text("none")
                }
            }
            if let p = f.program {
                row("program") {
                    HStack {
                        Text("/\(p.key)" + (p.subtype.map { " /\($0)" } ?? ""))
                        if let s = p.stream { ObjLink(model: model, id: s) }
                        if let format = p.format {
                            Text(programText(p, format))
                                .foregroundStyle(.secondary)
                        } else {
                            Text("unreadable").foregroundStyle(.red)
                        }
                    }
                }
            } else if f.kind != .type3 {
                row("program") { Text("not embedded").foregroundStyle(.orange) }
            }
            if let d = f.descriptor { row("descriptor") { ObjLink(model: model, id: d) } }
            if let flags = f.flags {
                row("flags") { Text("\(flags)" + (f.flagNames.isEmpty ? "" : " (\(f.flagNames.joined(separator: ", ")))")) }
            }
            row("ascent / descent") { Text("\(numText(f.ascent)) / \(numText(f.descent))") }
            if let m = f.fontMatrix { row("FontMatrix") { Text("[\(m.map(numText).joined(separator: " "))]") } }
            if f.approximate { row("widths") { Text("partly guessed").foregroundStyle(.orange) } }
        }
        .font(.caption)
        .textSelection(.enabled)
    }

    private func programText(_ p: FontProgramInfo, _ format: String) -> String {
        var parts = [format, bytesText(p.len)]
        if let g = p.glyphs { parts.append("\(g.formatted()) glyphs") }
        if let u = p.unitsPerEm { parts.append("\(u) units/em") }
        if p.cidKeyed { parts.append("CID-keyed") }
        return parts.joined(separator: " · ")
    }

    private func row(_ k: String, @ViewBuilder _ v: () -> some View) -> some View {
        GridRow {
            Text(k).foregroundStyle(.secondary)
            v()
        }
    }
}

/// The text the font shows, found by a job over the pages that use it.
private struct FontUses: View {
    let model: DocumentModel
    let f: FontDetail
    @State private var error: String?

    private var job: JobInfo? {
        model.latestJob { if case .fontUses(let font) = $0 { font == f.id } else { false } }
    }

    var body: some View {
        Loaded(model.generation, fetch: {
            try await model.backend.fontUses(model.doc, id: f.id, range: ListRange(offset: 0, limit: 0))
        }) { u in
            content(u)
        }
    }

    @ViewBuilder private func content(_ u: FontUsesView) -> some View {
        let total = u.uses.total
        let running = job?.state == .running
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Text drawn with this font").font(.headline)
                if u.scanned {
                    Text(
                        "\(total.formatted()) \(total == 1 ? "operation" : "operations") on \(u.pages.formatted()) of \(u.pagesScanned.formatted()) pages · \(u.codes.count.formatted()) distinct codes"
                    )
                    .font(.caption).foregroundStyle(.secondary)
                }
            }
            if !u.scanned {
                HStack {
                    Button("Find Text") { start() }.disabled(running)
                    Text("Interprets the pages using the font (every page until the inventory is built), forms included.")
                        .font(.caption).foregroundStyle(.secondary)
                    if let job, running {
                        Button("Cancel", role: .cancel) { Task { await model.cancelJob(job.job) } }
                    }
                }
                .controlSize(.small)
                if let job {
                    ProgressView(value: Double(job.done), total: Double(max(job.total, 1))).controlSize(.small)
                    Text("\(job.done.formatted()) / \(job.total.formatted()) pages\(job.state == .cancelled ? " · cancelled" : "")")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            if u.scanned && total > 0 {
                let backend = model.backend
                let doc = model.doc
                let id = f.id
                PagedRows<FontUsePage, _>(
                    model: model, total: Int(total),
                    fetch: { try await backend.fontUses(doc, id: id, range: $0).uses }
                ) { x, _ in
                    HStack(spacing: 8) {
                        Text("p. \(x.page + 1) #\(x.op)").monospaced().foregroundStyle(.secondary)
                        Text(x.text.isEmpty ? "(no text)" : x.text).lineLimit(1)
                            .foregroundStyle(x.hidden ? .secondary : .primary)
                        if x.hidden { Badge("invisible") }
                        Spacer()
                        if let form = x.form {
                            LinkButton("in form \(form.objText) #\(x.formOp ?? 0)") {
                                model.nav.open(.content(.stream(id: form), op: x.formOp))
                            }
                            .help("The operation inside the form XObject")
                        }
                    }
                    .font(.caption)
                    .contentShape(.rect)
                    .onTapGesture { model.nav.open(.content(.page(index: x.page), op: x.op)) }
                }
                .frame(height: min(Double(total) * 24, 8 * 24) + 4)
                .border(.separator)
            } else if u.scanned {
                Text("No page shows text with it.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func start() {
        error = nil
        Task {
            do {
                try await model.startJob(.fontUses(font: f.id))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// One entry of the glyph table: a code, or a glyph of the program.
private enum GlyphEntry: Equatable {
    case code(GlyphView)
    case program(ProgramGlyph)

    var key: String {
        switch self {
        case .code(let g): "c\(g.bytes)"
        case .program(let g): "g\(g.gid)"
        }
    }

    var outline: String? {
        switch self {
        case .code(let g): g.outline
        case .program(let g): g.outline
        }
    }

    var bbox: Rect? {
        switch self {
        case .code(let g): g.bbox
        case .program(let g): g.bbox
        }
    }

    var width: Double {
        switch self {
        case .code(let g): g.width
        case .program(let g): g.width
        }
    }

    var text: String? {
        switch self {
        case .code(let g): g.text
        case .program(let g): g.text
        }
    }

    var name: String? {
        switch self {
        case .code(let g): g.name
        case .program(let g): g.name
        }
    }

    var codes: [String] {
        switch self {
        case .code(let g): [g.bytes]
        case .program(let g): g.codes
        }
    }
}

/// The glyph table: by code (what the font can show) or, for an embedded
/// program, by glyph index (everything it holds), fetched by pages as
/// cells come into view.
private struct GlyphTable: View {
    let model: DocumentModel
    let f: FontDetail
    @State private var byProgram = false
    @State private var picked: GlyphEntry?
    @State private var shown: [String: UInt64]?

    var body: some View {
        let programGlyphs = f.glyphSource == .embedded ? Int(f.program?.glyphs ?? 0) : 0
        let listing = programGlyphs > 0 && byProgram
        let total = listing ? programGlyphs : Int(f.codes)
        VStack(alignment: .leading, spacing: 6) {
            if total == 0 {
                Text("No glyphs to list.").font(.caption).foregroundStyle(.secondary)
            } else {
                HStack {
                    Text("Glyphs").font(.headline)
                    if programGlyphs > 0 {
                        Picker("Glyphs", selection: $byProgram) {
                            Text("\(f.codes.formatted()) codes").tag(false)
                            Text("\(programGlyphs.formatted()) in the program").tag(true)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .fixedSize()
                        .controlSize(.small)
                    }
                    if let shown {
                        Label("shown in the document (\(shown.count.formatted()) codes)", systemImage: "circle.fill")
                            .foregroundStyle(.secondary)
                            .tint(.teal)
                    }
                    Text(
                        listing
                            ? "glyph index → codes that select it · dimmed: no code selects it"
                            : (programGlyphs > 0 ? "" : "\(total.formatted()) codes · ") + "code → glyph → text"
                    )
                    .foregroundStyle(.secondary)
                }
                .font(.caption)
                if let picked { PickedGlyph(model: model, f: f, e: picked, shown: timesShown(picked)) }
                Group {
                    if listing {
                        GlyphGrid<ProgramGlyphPage>(
                            model: model, f: f, total: total, picked: picked, shown: shown,
                            fetch: { [backend = model.backend, doc = model.doc, id = f.id] in
                                try await backend.programGlyphs(doc, id: id, range: $0)
                            },
                            entry: { .program($0) }, pick: { picked = $0 })
                    } else {
                        GlyphGrid<GlyphViewPage>(
                            model: model, f: f, total: total, picked: picked, shown: shown,
                            fetch: { [backend = model.backend, doc = model.doc, id = f.id] in
                                try await backend.glyphs(doc, id: id, range: $0)
                            },
                            entry: { .code($0) }, pick: { picked = $0 })
                    }
                }
                .id(listing)
            }
        }
        .onChange(of: byProgram) { picked = nil }
        .task(id: model.generation) {
            let u = try? await model.backend.fontUses(model.doc, id: f.id, range: ListRange(offset: 0, limit: 0))
            shown = u.flatMap { $0.scanned ? Dictionary($0.codes.map { ($0.bytes, $0.count) }) { a, _ in a } : nil }
        }
    }

    private func timesShown(_ e: GlyphEntry) -> UInt64? {
        guard let shown else { return nil }
        return e.codes.reduce(0) { $0 + (shown[$1] ?? 0) }
    }
}

private struct GlyphGrid<P: ListPage>: View {
    let model: DocumentModel
    let f: FontDetail
    let total: Int
    let picked: GlyphEntry?
    let shown: [String: UInt64]?
    let entry: (P.Item) -> GlyphEntry
    let pick: (GlyphEntry) -> Void
    @State private var list: PagedList<P>

    init(
        model: DocumentModel, f: FontDetail, total: Int, picked: GlyphEntry?, shown: [String: UInt64]?,
        fetch: @escaping @Sendable (ListRange) async throws -> P, entry: @escaping (P.Item) -> GlyphEntry,
        pick: @escaping (GlyphEntry) -> Void
    ) {
        self.model = model
        self.f = f
        self.total = total
        self.picked = picked
        self.shown = shown
        self.entry = entry
        self.pick = pick
        _list = State(initialValue: PagedList(total: total, fetch: fetch))
    }

    var body: some View {
        let rows = (total + 7) / 8
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 76, maximum: 76), spacing: 0)], spacing: 0) {
                ForEach(0..<total, id: \.self) { i in
                    Group {
                        if let item = list.item(i) {
                            let e = entry(item)
                            GlyphCell(f: f, e: e, shown: shown.map { s in e.codes.reduce(0) { $0 + (s[$1] ?? 0) } },
                                      active: picked?.key == e.key)
                                .onTapGesture { pick(e) }
                        } else {
                            ProgressView().controlSize(.mini)
                        }
                    }
                    .frame(width: 76, height: 92)
                    .onAppear { list.need(i) }
                }
            }
        }
        .frame(height: min(Double(rows) * 92 + 2, 560))
        .border(.separator)
        .onChange(of: model.generation) { list.reset() }
    }
}

private struct GlyphCell: View {
    let f: FontDetail
    let e: GlyphEntry
    let shown: UInt64?
    let active: Bool

    var body: some View {
        let procedure: ObjRef? = if case .code(let g) = e { g.procedure } else { nil }
        let empty: Bool = {
            if e.outline == nil && procedure == nil { return true }
            if case .program(let g) = e { return g.codes.isEmpty }
            return false
        }()
        let head: String =
            switch e {
            case .code(let g): g.bytes
            case .program(let g):
                "#\(g.gid)" + (g.codes.first.map { " · \($0)\(g.codes.count > 1 ? "+" : "")" } ?? "")
            }
        VStack(spacing: 1) {
            Text(head).font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary).lineLimit(1)
            ZStack {
                if e.outline != nil {
                    GlyphShape(f: f, e: e)
                } else if procedure != nil {
                    Text("proc").font(.system(size: 10)).foregroundStyle(.purple)
                }
            }
            .frame(width: 44, height: 44)
            .frame(maxHeight: .infinity)
            Text(e.text.map(escaped) ?? e.name ?? "").font(.system(size: 10)).lineLimit(1).help(e.name ?? "")
        }
        .padding(3)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .opacity(empty ? 0.4 : 1)
        .background(active ? Color.accentColor.opacity(0.2) : (shown ?? 0) > 0 ? Color.teal.opacity(0.1) : .clear)
        .overlay(Rectangle().strokeBorder(.separator.opacity(0.5), lineWidth: 0.5))
        .contentShape(.rect)
        .help(shown.map { $0 > 0 ? "shown \($0.formatted()) times" : "" } ?? "")
    }
}

/// Control characters and such escaped, like JSON.
private func escaped(_ s: String) -> String {
    s.unicodeScalars.map { c in
        c.value < 0x20 || c.value == 0x7f ? String(format: "\\u%04x", c.value) : String(c)
    }.joined()
}

/// The glyph in a box from the font's descent to its ascent, with the
/// baseline and advance marked.
private struct GlyphShape: View {
    let f: FontDetail
    let e: GlyphEntry

    var body: some View {
        Canvas { ctx, size in
            let top = max(f.ascent != 0 ? f.ascent : 800, e.bbox?.y1 ?? 0)
            let bottom = min(f.descent != 0 ? f.descent : -200, e.bbox?.y0 ?? 0)
            let x0 = min(0, e.bbox?.x0 ?? 0)
            let x1 = max(e.width, e.bbox?.x1 ?? 0, 1)
            let pad = (top - bottom) * 0.06
            let w = x1 - x0 + 2 * pad
            let h = top - bottom + 2 * pad
            let k = min(size.width / w, size.height / h)
            // Glyph space (y up) to the view, centered.
            let tx = (size.width - w * k) / 2 - (x0 - pad) * k
            let ty = (size.height - h * k) / 2 + (top + pad) * k
            let m = CGAffineTransform(a: k, b: 0, c: 0, d: -k, tx: tx, ty: ty)
            var base = Path()
            base.move(to: CGPoint(x: x0 - pad, y: 0).applying(m))
            base.addLine(to: CGPoint(x: x1 + pad, y: 0).applying(m))
            ctx.stroke(base, with: .color(.blue.opacity(0.5)), lineWidth: 0.5)
            if e.width > 0 {
                var adv = Path()
                adv.move(to: CGPoint(x: e.width, y: top).applying(m))
                adv.addLine(to: CGPoint(x: e.width, y: bottom).applying(m))
                ctx.stroke(adv, with: .color(.gray), style: StrokeStyle(lineWidth: 0.5, dash: [3, 2]))
            }
            if let o = e.outline, let p = glyphPath(o) {
                ctx.fill(Path(p).applying(m), with: .color(.primary))
            }
        }
    }
}

/// The picked glyph, large, with everything known about it.
private struct PickedGlyph: View {
    let model: DocumentModel
    let f: FontDetail
    let e: GlyphEntry
    let shown: UInt64?

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            Group {
                if e.outline != nil {
                    GlyphShape(f: f, e: e)
                } else {
                    Text("no outline").font(.caption).foregroundStyle(.secondary)
                }
            }
            .frame(width: 120, height: 120)
            .background(.background)
            .border(.separator)
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 2) {
                switch e {
                case .code(let g):
                    field("code", "<\(g.bytes)>")
                    if let c = g.cid { field("CID", "\(c)") }
                    if let gid = g.gid { field("glyph", "\(gid)") }
                case .program(let g):
                    field("glyph", "\(g.gid)")
                    field("codes", g.codes.isEmpty ? "none: the font cannot show it" : g.codes.map { "<\($0)>" }.joined(separator: " "))
                }
                if let shown { field("shown", shown > 0 ? "\(shown.formatted()) times" : "never") }
                if let n = e.name { field("name", "/\(n)") }
                field("text", e.text.map { t in
                    "\"\(escaped(t))\" " + t.unicodeScalars.map { String(format: "U+%04X", $0.value) }.joined(separator: " ")
                } ?? "unknown")
                if case .program = e {
                    field("width", "\(numText(e.width)) (program)")
                } else {
                    field("width", numText(e.width))
                }
                if let b = e.bbox { field("bbox", b.text) }
                if case .code(let g) = e, let p = g.procedure {
                    GridRow {
                        Color.clear.frame(width: 0, height: 0)
                        LinkButton("procedure \(p.objText) →") { model.nav.open(.content(.stream(id: p), op: nil)) }
                            .help("The glyph procedure's operations")
                    }
                }
            }
            .font(.caption)
            .textSelection(.enabled)
        }
        .padding(8)
        .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 6))
    }

    private func field(_ k: String, _ v: String) -> some View {
        GridRow {
            Text(k).foregroundStyle(.secondary)
            Text(v).monospaced()
        }
    }
}

#if DEBUG
#Preview("Glyph cells") {
    let path = URL.cachesDirectory.appending(path: "fixtures/fonts.pdf").path
    let open = try! Backend.shared.sendNow(.open(source: .path(path: path), password: nil, recover: false))
    guard case .doc(let s) = open.body else { fatalError() }
    let doc = s.info.doc
    let font = ObjRef(num: 21, generation: 0)
    guard case .font(let f) = try! Backend.shared.sendNow(.font(doc: doc, id: font)).body,
        case .glyphs(let g) = try! Backend.shared.sendNow(
            .glyphs(doc: doc, id: font, range: ListRange(offset: 60, limit: 8))).body
    else { fatalError() }
    return VStack {
        HStack(spacing: 0) {
            ForEach(g.items, id: \.code) { GlyphCell(f: f, e: .code($0), shown: nil, active: false).frame(width: 76, height: 92) }
        }
        PickedGlyph(model: DocumentModel(summary: s), f: f, e: .code(g.items[5]), shown: 3)
    }
    .padding()
}
#endif
