import PdfInspectorKit
import SwiftUI

/// The outline (bookmarks), expanded lazily (the web `OutlinePanel`).
struct OutlinePanel: View {
    let model: DocumentModel

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                OutlineItems(model: model, parent: nil, depth: 0)
            }
            .padding(.vertical, 4)
        }
    }
}

private struct OutlineItems: View {
    let model: DocumentModel
    let parent: ObjRef?
    let depth: Int
    @State private var items: [OutlineItem] = []
    @State private var total: UInt64?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(items, id: \.id) { it in
                OutlineRow(model: model, item: it, depth: depth)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).padding(.leading, indent)
            } else if let total {
                if total == 0 && depth == 0 {
                    Text("No outline.").foregroundStyle(.secondary).padding(10)
                } else if UInt64(items.count) < total {
                    LinkButton("Show more (\(items.count.formatted()) of \(total.formatted()))") {
                        Task { await more() }
                    }
                    .font(.caption)
                    .padding(.leading, indent)
                }
            } else {
                ProgressView().controlSize(.mini).padding(.leading, indent)
            }
        }
        .task(id: model.generation) {
            items = []
            total = nil
            await more()
        }
    }

    private var indent: CGFloat { CGFloat(depth) * 16 + 20 }

    private func more() async {
        do {
            let p = try await model.backend.outline(
                model.doc, parent: parent, range: ListRange(offset: UInt64(items.count), limit: 200))
            items += p.items
            total = p.total
        } catch is CancellationError {
        } catch {
            self.error = error.localizedDescription
        }
    }
}

private struct OutlineRow: View {
    let model: DocumentModel
    let item: OutlineItem
    let depth: Int
    @State private var open: Bool

    init(model: DocumentModel, item: OutlineItem, depth: Int) {
        self.model = model
        self.item = item
        self.depth = depth
        // Positive /Count: shown open by viewers.
        _open = State(initialValue: (item.count ?? 0) > 0 && depth < 2)
    }

    private var page: UInt32? {
        if case .dest(let d) = item.link { d.pageIndex } else { nil }
    }

    var body: some View {
        let expandable = item.hasChildren && depth < 32
        let active = model.nav.target == .object(item.id)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 4) {
                Button {
                    open.toggle()
                } label: {
                    Image(systemName: open ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                        .frame(width: 12)
                        .opacity(expandable ? 1 : 0)
                }
                .buttonStyle(.plain)
                .disabled(!expandable)
                Text(item.title.isEmpty ? "(untitled)" : item.title)
                    .bold(item.flags & 2 != 0)
                    .italic(item.flags & 1 != 0 || item.title.isEmpty)
                    .foregroundStyle(titleColor)
                    .lineLimit(1)
                Text(page.map { "p. \($0 + 1)" } ?? item.link?.text ?? "")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                LinkButton("\(item.id.num)") { model.nav.open(.object(item.id)) }
                    .monospaced()
                    .font(.caption)
            }
            .font(.callout)
            .padding(.leading, CGFloat(depth) * 16 + 4)
            .padding(.trailing, 8)
            .padding(.vertical, 2)
            .background(active ? Color.accentColor.opacity(0.2) : .clear)
            .contentShape(.rect)
            .onTapGesture {
                model.nav.open(page.map { .page($0) } ?? .object(item.id))
            }
            .help(item.link?.text ?? "no destination")
            if open && expandable {
                OutlineItems(model: model, parent: item.id, depth: depth + 1)
            }
        }
    }

    private var titleColor: Color {
        guard let c = item.color, c.count == 3, c.contains(where: { $0 > 0 }) else { return .primary }
        return Color(red: c[0], green: c[1], blue: c[2])
    }
}

/// Cross-reference entries of the merged table or of one revision (the web
/// `XrefPanel`).
struct XrefPanel: View {
    let model: DocumentModel
    @State private var revision: UInt32?

    var body: some View {
        let s = model.summary
        let r = revision.flatMap { i in s.revisions.first { $0.index == i } }
        let doc = model.doc
        let backend = model.backend
        let rev = revision
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Picker("Table", selection: $revision) {
                    Text("Merged table").tag(UInt32?.none)
                    ForEach(s.revisions, id: \.index) { r in
                        Text("#\(r.index) \(r.kind.text) @\(r.offset.formatted())\(r.index == 0 ? " (newest)" : "")")
                            .tag(Optional(r.index))
                    }
                }
                .labelsHidden()
                if let r {
                    HStack {
                        LinkButton(r.span.text) { model.nav.showBytes(r.span, reveal: true) }
                        if let x = r.xrefStream {
                            LinkButton(x.objText) { model.nav.open(.object(x)) }
                        }
                    }
                    .font(.caption)
                }
            }
            .padding(6)
            Divider()
            PagedRows<XrefRowPage, _>(
                model: model, total: Int(r?.entries ?? s.xrefEntries),
                fetch: { try await backend.xref(doc, revision: rev, range: $0) }
            ) { x, _ in
                let free: Bool = if case .free = x.entry { true } else { false }
                Button {
                    model.nav.open(.object(ObjRef(num: x.num, generation: x.entry.generation)))
                } label: {
                    HStack {
                        Text("\(x.num)").frame(width: 64, alignment: .trailing)
                        Text("\(x.entry.generation)").foregroundStyle(.secondary).frame(width: 48, alignment: .leading)
                        Text("\(x.entry.letter) \(x.entry.text)")
                        Spacer()
                    }
                    .monospaced()
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .disabled(free)
                .font(.callout)
            }
            .id(revision)
        }
    }
}

extension XrefKind {
    var text: String {
        switch self {
        case .table: "table"
        case .stream: "stream"
        case .hybrid: "hybrid"
        case .recovered: "recovered"
        }
    }
}

extension XrefEntry {
    var letter: String {
        switch self {
        case .inUse: "n"
        case .free: "f"
        case .compressed: "c"
        case .edited: "e"
        }
    }
}

/// Schema validation of the whole document or of the current object and
/// what it refers to (the web `ValidatePanel`).
struct ValidatePanel: View {
    let model: DocumentModel
    @State private var error: String?

    private var job: JobInfo? {
        model.latestJob { if case .validate = $0 { true } else { false } }
    }

    var body: some View {
        let job = job
        let running = job?.state == .running
        let target: ObjRef? = if case .object(let id) = model.nav.target { id } else { nil }
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Checks objects against the Arlington PDF Model, following typed references.")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Whole document") { start(nil) }
                    Button(target.map { "From \($0.objText)" } ?? "From object") { start(target) }
                        .disabled(target == nil)
                    if let job, running {
                        Button("Cancel", role: .cancel) { Task { await model.cancelJob(job.job) } }
                    }
                }
                .controlSize(.small)
                .disabled(running)
                if let job {
                    ProgressView(value: Double(job.done), total: Double(max(job.total, job.done, 1)))
                        .controlSize(.small)
                    HStack(spacing: 6) {
                        Text(summary(job)).font(.caption).foregroundStyle(.secondary)
                        if case .validate(_, let errors, let warnings, let infos, _)? = job.result {
                            Badge("\(errors.formatted()) errors", color: .red)
                            Badge("\(warnings.formatted()) warnings", color: .orange)
                            Badge("\(infos.formatted()) info", color: .blue)
                        }
                    }
                    if case .validate(_, let e, let w, let i, let kept)? = job.result, kept < e + w + i {
                        Text("Showing the first \(kept.formatted()).").font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
            }
            .padding(8)
            Divider()
            if let job, case .validate(_, _, _, _, let kept)? = job.result {
                let backend = model.backend
                let id = job.job
                PagedRows<DiagnosticInfoPage, _>(
                    model: model, total: Int(kept), empty: "No issues found.",
                    fetch: { try await backend.jobDiagnostics(id, range: $0) }
                ) { d, _ in
                    Button {
                        model.nav.open(d.obj.map { .object($0) } ?? .trailer)
                        if let s = d.span { model.nav.showBytes(s) }
                    } label: {
                        HStack(alignment: .firstTextBaseline) {
                            Badge(d.code, color: d.severity.color)
                            VStack(alignment: .leading) {
                                Text(d.message).lineLimit(2).help(d.message)
                                Text((d.obj?.objText ?? "trailer") + (d.path ?? ""))
                                    .monospaced().foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .font(.caption)
                }
                .id(job.job)
            } else {
                Spacer()
            }
        }
    }

    private func summary(_ job: JobInfo) -> String {
        var s = "Document: "
        if case .validate(let root?) = job.kind { s = "From \(root.objText): " }
        s += "\(job.done.formatted()) objects"
        if job.state == .running { s += " · \((job.total - min(job.done, job.total)).formatted()) queued" }
        if job.state == .cancelled { s += " · cancelled" }
        return s
    }

    private func start(_ root: ObjRef?) {
        error = nil
        Task {
            do {
                try await model.startJob(.validate(root: root))
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

/// What the object list is filtered by, kept while the window is open.
struct ObjectPicks: Equatable {
    /// Schema type (Arlington).
    var schema: String??
    var type: String??
    var subtype: String??
    var kind: String?
    var unreferenced = false

    var byClass: Bool { schema != nil || type != nil || subtype != nil || kind != nil }
    var any: Bool { byClass || unreferenced }

    enum Field { case schema, type, subtype, kind }

    func matches(_ c: ObjectClass, skip: Field? = nil) -> Bool {
        (skip == .schema || schema == nil || schema! == c.schemaType)
            && (skip == .type || type == nil || type! == c.typeName)
            && (skip == .subtype || subtype == nil || subtype! == c.subtype)
            && (skip == .kind || kind == nil || kind! == c.kind)
    }

    /// The values of one field among the classes the other picks leave,
    /// with their counts, most first.
    func options(_ all: [ObjectClass], _ field: Field) -> [(value: String?, count: UInt64)] {
        var counts: [String?: UInt64] = [:]
        for c in all where matches(c, skip: field) {
            let v: String? =
                switch field {
                case .schema: c.schemaType
                case .type: c.typeName
                case .subtype: c.subtype
                case .kind: c.kind
                }
            counts[v, default: 0] += c.count
        }
        return counts.map { ($0.key, $0.value) }
            .sorted { $0.count != $1.count ? $0.count > $1.count : ($0.value ?? "") < ($1.value ?? "") }
    }
}

/// The objects by number, or those a filter picks (the web `ObjectList`).
struct ObjectsPanel: View {
    let model: DocumentModel
    @Bindable var layout: WindowLayout
    @State private var goto = ""
    @State private var classes: [ObjectClass]?

    private var filter: ObjectFilter? {
        let p = layout.objectPicks
        guard model.summary.inventory != nil, let classes, p.any else { return nil }
        return ObjectFilter(
            classes: p.byClass ? classes.filter { p.matches($0) } : nil, unreferenced: p.unreferenced)
    }

    var body: some View {
        let total = Int(model.summary.xrefEntries)
        let filter = filter
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                TextField("Go to object #", text: $goto)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        if let n = UInt32(goto.trimmingCharacters(in: .whitespaces)) {
                            model.nav.open(.object(ObjRef(num: n, generation: 0)))
                        }
                    }
                if filter == nil {
                    Text("\(total.formatted()) entries").font(.caption).foregroundStyle(.secondary)
                }
            }
            .controlSize(.small)
            .padding(6)
            if let stats = model.summary.inventory {
                if let classes { pickers(classes, stats) }
            } else {
                HStack {
                    Text("Filter by schema type, /Type, /Subtype, kind or references:")
                        .font(.caption).foregroundStyle(.secondary)
                    LinkButton("Build inventory") { Task { try? await model.startJob(.inventory) } }
                        .font(.caption)
                }
                .padding(.horizontal, 6)
                .padding(.bottom, 6)
            }
            Divider()
            if let filter {
                Loaded(FilterKey(filter: filter, generation: model.generation), fetch: {
                    try await model.backend.objects(model.doc, range: ListRange(offset: 0, limit: 0), filter: filter)
                        .total
                }) { shown in
                    VStack(spacing: 0) {
                        Text("\(shown.formatted()) of \(total.formatted())")
                            .font(.caption).foregroundStyle(.secondary).padding(4)
                        list(total: Int(shown), filter: filter)
                    }
                }
            } else {
                list(total: total, filter: nil)
            }
        }
        .task(id: TaskKey(inventory: model.summary.inventory != nil, generation: model.generation)) {
            if model.summary.inventory != nil {
                classes = try? await model.backend.objectClasses(model.doc)
            }
        }
    }

    private struct FilterKey: Equatable {
        let filter: ObjectFilter
        let generation: Int
    }

    private struct TaskKey: Equatable {
        let inventory: Bool
        let generation: Int
    }

    @ViewBuilder private func pickers(_ classes: [ObjectClass], _ stats: InventoryStats) -> some View {
        let p = layout.objectPicks
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 4) {
                picker("Schema type", .schema, classes, \.schema, name: false)
                picker("/Type", .type, classes, \.type, name: true)
            }
            HStack(spacing: 4) {
                picker("/Subtype", .subtype, classes, \.subtype, name: true)
                Picker("Kind", selection: $layout.objectPicks.kind) {
                    Text("Kind: any").tag(String?.none)
                    ForEach(p.options(classes, .kind), id: \.value) { o in
                        Text("\(o.value ?? "") · \(o.count.formatted())").tag(o.value)
                    }
                }
                .labelsHidden()
            }
            HStack {
                Toggle("Unreferenced (\(stats.unreferenced.formatted()))", isOn: $layout.objectPicks.unreferenced)
                Spacer()
                if p.any {
                    Button("Clear") { layout.objectPicks = ObjectPicks() }.buttonStyle(.link)
                }
            }
        }
        .controlSize(.small)
        .font(.caption)
        .padding(.horizontal, 6)
        .padding(.bottom, 6)
    }

    private func picker(
        _ title: String, _ field: ObjectPicks.Field, _ classes: [ObjectClass],
        _ key: WritableKeyPath<ObjectPicks, String??>, name: Bool
    ) -> some View {
        Picker(title, selection: $layout.objectPicks[dynamicMember: key]) {
            Text("\(title): any").tag(String??.none)
            ForEach(layout.objectPicks.options(classes, field), id: \.value) { o in
                Text("\(o.value.map { name ? "/\($0)" : $0 } ?? "(none)") · \(o.count.formatted())")
                    .tag(String??.some(o.value))
            }
        }
        .labelsHidden()
    }

    private func list(total: Int, filter: ObjectFilter?) -> some View {
        let doc = model.doc
        let backend = model.backend
        let current: UInt32? = if case .object(let id) = model.nav.target { id.num } else { nil }
        let empty =
            layout.objectPicks.unreferenced && !layout.objectPicks.byClass
            ? "Every object is referred to." : "No object matches."
        return PagedRows<ObjectSummaryPage, _>(
            model: model, total: total, empty: empty,
            fetch: { try await backend.objects(doc, range: $0, filter: filter) }
        ) { o, _ in
            let id = ObjRef(num: o.num, generation: o.entry.generation)
            let free: Bool = if case .free = o.entry { true } else { false }
            Button {
                model.nav.open(.object(id))
            } label: {
                HStack {
                    Text("\(o.num) \(id.generation)").monospaced().frame(width: 72, alignment: .trailing)
                        .foregroundStyle(free ? .secondary : .primary)
                    if o.error != nil {
                        Badge("error", color: .red).help(o.error ?? "")
                    } else if let k = o.kind {
                        Text(k).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
                    }
                    Text(o.schemaType ?? ((o.typeName.map { "/\($0)" } ?? "") + (o.subtype.map { " /\($0)" } ?? "")))
                        .fontWeight(.medium)
                        .lineLimit(1)
                        .help([o.typeName.map { "/\($0)" }, o.subtype.map { "/\($0)" }].compactMap { $0 }
                            .joined(separator: " "))
                    Spacer()
                    Text(o.entry.text).foregroundStyle(.secondary).monospaced()
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(free)
            .font(.callout)
            .listRowBackground(o.num == current ? Color.accentColor.opacity(0.2) : nil)
        }
        .id(filter)
    }
}
