import PdfInspectorKit
import SwiftUI

/// Where the current view sits: the chain of references from the trailer to
/// the object it shows (shortest, preferring forward links), then the view
/// (the web `Breadcrumbs`).
struct Breadcrumbs: View {
    let model: DocumentModel
    @State private var info: Info?

    private struct Crumb {
        /// Entry of the previous crumb that leads here: `/Pages`, `/Kids[3]`.
        var key: String?
        let label: String
        var help: String?
        /// Where clicking goes (none: the current view).
        var to: Target?
    }

    /// What the crumbs are made of, read for the current target.
    private struct Info {
        let id: ObjRef?
        let page: PageDetail?
        let schema: ObjectSchema?
        let path: RefPath?
    }

    private struct Key: Equatable {
        let target: Target
        let inventory: Bool
        let generation: Int

        init(_ m: DocumentModel) {
            // The selected operation does not move the crumbs.
            if case .content(let c, _) = m.nav.target { target = .content(c, op: nil) } else { target = m.nav.target }
            inventory = m.summary.inventory != nil
            generation = m.generation
        }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                HStack(spacing: 4) {
                    ForEach(Array(crumbs.enumerated()), id: \.offset) { i, c in
                        if i > 0 { Text("›").foregroundStyle(.secondary) }
                        if i == 1 { gap }
                        if let k = c.key { Text(k).monospaced().foregroundStyle(.secondary) }
                        if let to = c.to {
                            LinkButton(c.label) { model.nav.open(to) }.help(c.help ?? "")
                        } else {
                            Text(c.label).fontWeight(.semibold).help(c.help ?? "")
                        }
                    }
                    Color.clear.frame(width: 1, height: 1).id("end")
                }
                .font(.caption)
                .lineLimit(1)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
            }
            .scrollIndicators(.never)
            .onChange(of: crumbs.count) { proxy.scrollTo("end") }
        }
        .task(id: Key(model)) { await load() }
    }

    private func targetPage(_ t: Target) -> UInt32? {
        switch t {
        case .page(let i), .content(.page(let i), _): i
        default: nil
        }
    }

    private func load() async {
        let t = model.nav.target
        let page: PageDetail? =
            if let i = targetPage(t) { try? await model.backend.page(model.doc, index: i) } else { nil }
        let id: ObjRef? =
            switch t {
            case .object(let id), .font(let id), .content(.stream(let id), _): id
            default: page?.id
            }
        var schema: ObjectSchema?
        var path: RefPath?
        if let id {
            schema = try? await model.backend.schema(model.doc, root: .object(id: id))
            if model.summary.inventory != nil { path = try? await model.backend.refPath(model.doc, id: id) }
        }
        info = Info(id: id, page: page, schema: schema, path: path)
    }

    private func objLabel(_ id: ObjRef) -> String { "\(id.num) \(id.generation)" }

    /// Arlington names are long; `/Type` guesses come with a slash.
    private func typeLabel(_ t: String?) -> String? {
        t.map { $0.hasPrefix("/") ? String($0.dropFirst()) : $0 }
    }

    @ViewBuilder private var gap: some View {
        if let info, info.id != nil, info.path?.steps.isEmpty ?? true, info.page == nil {
            if let p = info.path, !p.reachable {
                Badge("unreachable", color: .orange).help("No chain of references from the trailer leads to this object.")
                Text("›").foregroundStyle(.secondary)
            } else if model.summary.inventory == nil {
                Button("…") { Task { try? await model.startJob(.inventory) } }
                    .buttonStyle(.borderless)
                    .help("Build the inventory (one pass over the file) to show how the trailer reaches this object")
                Text("›").foregroundStyle(.secondary)
            }
        }
    }

    private var crumbs: [Crumb] {
        let t = model.nav.target
        switch t {
        case .document: return [Crumb(label: "Document")]
        case .trailer: return [Crumb(label: "Trailer")]
        default: break
        }
        guard let info, let id = info.id else {
            return targetPage(t).map { [Crumb(label: "Page \($0 + 1)")] } ?? []
        }
        var out: [Crumb] = []
        let steps = info.path?.steps ?? []
        if !steps.isEmpty {
            for (i, s) in steps.enumerated() {
                switch s.from {
                case .trailer:
                    out.append(Crumb(label: "Trailer", to: .trailer))
                case .object(let f):
                    out.append(Crumb(
                        key: i > 0 ? steps[i - 1].key : nil, label: typeLabel(s.typeName) ?? objLabel(f),
                        help: f.objText, to: .object(f)))
                }
            }
        } else if let page = info.page {
            // Known without the inventory: the page tree.
            out.append(Crumb(label: "Trailer", to: .trailer))
            if let root = model.summary.root { out.append(Crumb(key: "/Root", label: "Catalog", to: .object(root))) }
            for (i, a) in page.ancestors.enumerated() {
                out.append(Crumb(key: i == 0 ? "/Pages" : "/Kids", label: "Pages", help: a.objText, to: .object(a)))
            }
        } else {
            out.append(Crumb(label: "Trailer", to: .trailer))
        }
        let lastKey = steps.last?.key ?? ((info.page?.ancestors.isEmpty ?? true) ? nil : "/Kids")
        let isObject: Bool = if case .object = t { true } else { false }
        out.append(Crumb(
            key: lastKey, label: typeLabel(info.schema?.typeName) ?? objLabel(id), help: id.objText,
            to: isObject ? nil : .object(id)))
        switch t {
        case .page(let i): out.append(Crumb(label: "Page \(pageText(i, label: info.page?.label))"))
        case .content: out.append(Crumb(label: "Operations"))
        case .font: out.append(Crumb(label: "Font"))
        default: break
        }
        return out
    }
}
