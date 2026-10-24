import PdfInspectorKit
import SwiftUI

/// What the specification says about an operator (Resources/OperatorDocs.json,
/// made from the web UI's table by scripts/operator-docs.mjs).
struct OpDoc: Decodable {
    struct Operand: Decodable {
        let name: String
        let meaning: String
        /// Names of the values it can take, by value.
        let values: [String: String]?
    }

    /// Section of ISO 32000-2 defining it.
    let section: String
    let operands: [Operand]
    /// Takes as many operands as the color space has components
    /// (`pattern`: and then maybe a pattern name).
    let variadic: String?
    let text: String

    static let all: [String: OpDoc] = {
        guard let url = Bundle.main.url(forResource: "OperatorDocs", withExtension: "json"),
            let data = try? Data(contentsOf: url)
        else { return [:] }
        return (try? JSONDecoder().decode([String: OpDoc].self, from: data)) ?? [:]
    }()

    /// Operand names and meanings for the operands written, variadic color
    /// operators included.
    func operands(count: Int) -> [Operand] {
        guard let variadic else { return operands }
        return (0..<count).map { i in
            variadic == "pattern" && i == count - 1
                ? Operand(
                    name: "name",
                    meaning: "pattern in /Pattern of the resources (Pattern color space), when last and a name",
                    values: nil)
                : Operand(name: "c\(i + 1)", meaning: "color component", values: nil)
        }
    }
}

/// The selected operation of a content explained: what its operator does
/// and what each operand means (the web `OperatorDoc`).
struct OperatorDoc: View {
    let model: DocumentModel
    let content: ContentRef
    let op: UInt32

    var body: some View {
        Loaded(model.generation, fetch: {
            try await model.backend.contentOps(model.doc, content, range: ListRange(offset: UInt64(op), limit: 1))
                .items.first
        }) { o in
            if let o { doc(o) }
        }
    }

    @ViewBuilder private func doc(_ o: OpView) -> some View {
        let d = OpDoc.all[o.operator]
        let color = Category.color(o.category)
        let operands = d?.operands(count: o.operands.count) ?? []
        let rows = max(operands.count, o.operands.count)
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(o.operator).font(.headline.monospaced()).foregroundStyle(color)
                Badge(Category.label(o.category), color: color)
            }
            Text("#\(o.index)" + (d.map { " · ISO 32000-2 §\($0.section)" } ?? ""))
                .font(.caption).foregroundStyle(.secondary)
        }
        if let d {
            Text(d.text).font(.caption)
        } else {
            Text("Not an operator of the specification: a reader ignores it inside BX … EX, and reports an error elsewhere.")
                .font(.caption).foregroundStyle(.secondary)
        }
        if rows > 0 {
            SectionLabel("Operands")
            Grid(alignment: .topLeading, horizontalSpacing: 8, verticalSpacing: 4) {
                ForEach(0..<rows, id: \.self) { i in
                    let spec = i < operands.count ? operands[i] : nil
                    let value = i < o.operands.count ? o.operands[i] : nil
                    GridRow {
                        Text(spec?.name ?? "(extra)").monospaced().foregroundStyle(spec == nil ? .red : .primary)
                            .frame(width: 72, alignment: .leading)
                        VStack(alignment: .leading, spacing: 1) {
                            if let value {
                                HStack(spacing: 4) {
                                    Text(value).monospaced().padding(.horizontal, 3)
                                        .background(.quaternary.opacity(0.6), in: .rect(cornerRadius: 3))
                                    if let named = spec?.values?[value] { Text(named) }
                                }
                            } else {
                                Text("missing").foregroundStyle(.red)
                            }
                            if let spec { Text(spec.meaning).foregroundStyle(.secondary) }
                            if let vs = spec?.values {
                                Text(vs.sorted { $0.key < $1.key }.map { "\($0.key): \($0.value)" }.joined(separator: " · "))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .font(.caption)
            .textSelection(.enabled)
        }
    }
}

// MARK: Graphics state

nonisolated private func n(_ v: Double) -> String {
    v == v.rounded() && abs(v) < 1e15 ? String(Int64(v)) : String(format: "%g", (v * 10000).rounded() / 10000)
}

nonisolated private func list(_ v: [Double]) -> String { "[\(v.map(n).joined(separator: " "))]" }

nonisolated private func rectText(_ r: Rect?) -> String {
    r.map { "[\(n($0.x0)) \(n($0.y0)) \(n($0.x1)) \(n($0.y1))]" } ?? "none"
}

/// An sRGB approximation of a color, when its space tells how.
private func swatch(_ c: ColorView) -> Color? {
    let v = c.components.map { min(1, max(0, $0)) }
    let by = c.family == "ICCBased" || c.family.hasPrefix("Device") || c.family.hasPrefix("Cal") ? v.count : 0
    switch by {
    case 1: return Color(white: v[0])
    case 3 where c.family != "Lab": return Color(red: v[0], green: v[1], blue: v[2])
    case 4:
        let k = min(1, c.components[3])
        let f = { (x: Double) in (1 - min(1, x)) * (1 - k) }
        return Color(red: f(c.components[0]), green: f(c.components[1]), blue: f(c.components[2]))
    default: return nil
    }
}

nonisolated private func colorText(_ c: ColorView) -> String {
    let space = c.space == c.family ? c.space : "/\(c.space) (\(c.family))"
    return "\(space) \(list(c.components))" + (c.pattern.map { " /\($0)" } ?? "")
}

private struct Field {
    let label: String
    let text: (GraphicsState) -> String
    var color: ((GraphicsState) -> ColorView)?
    /// Only shown when set in either state.
    var optional: ((GraphicsState) -> Bool)?
}

private let fields: [Field] = [
    Field(label: "CTM", text: { list($0.ctm) }),
    Field(label: "clip", text: { rectText($0.clip) }),
    Field(label: "saved (q)", text: { "\($0.saved)" }),
    Field(label: "marked", text: { $0.marked.isEmpty ? "—" : $0.marked.map { "/\($0)" }.joined(separator: " › ") }),
    Field(label: "fill", text: { colorText($0.fill) }, color: { $0.fill }),
    Field(label: "stroke", text: { colorText($0.stroke) }, color: { $0.stroke }),
    Field(label: "alpha", text: { "fill \(n($0.fillAlpha)) · stroke \(n($0.strokeAlpha))" }),
    Field(label: "blend", text: { $0.blend + ($0.softMask.map { " · soft mask: \($0)" } ?? "") }),
    Field(
        label: "gs", text: { $0.extGstates.isEmpty ? "—" : $0.extGstates.map { "/\($0)" }.joined(separator: " ") },
        optional: { !$0.extGstates.isEmpty }),
    Field(label: "line", text: { g in
        "width \(n(g.lineWidth)) · cap \(g.lineCap) · join \(g.lineJoin) · miter \(n(g.miterLimit))"
            + (g.dash.isEmpty ? "" : " · dash \(list(g.dash)) \(n(g.dashPhase))")
    }),
    Field(label: "intent", text: { "\($0.intent) · flatness \(n($0.flatness))" }),
    Field(label: "font", text: { g in
        g.font.map { f in "/\(f.name) \(n(g.fontSize))" + (f.baseFont.map { " (\($0)\(f.subtype.map { ", \($0)" } ?? ""))" } ?? "") }
            ?? "none"
    }),
    Field(label: "text state", text: { g in
        "Tc \(n(g.charSpacing)) · Tw \(n(g.wordSpacing)) · Tz \(n(g.horizontalScale)) · TL \(n(g.leading)) · Tr \(g.renderMode) · Ts \(n(g.rise))"
    }),
    Field(label: "Tm", text: { $0.textMatrix.map(list) ?? "—" }, optional: { $0.inText }),
    Field(label: "Tlm", text: { $0.lineMatrix.map(list) ?? "—" }, optional: { $0.inText }),
    Field(label: "path", text: { rectText($0.path) }, optional: { $0.path != nil }),
    Field(label: "current point", text: { $0.currentPoint.map(list) ?? "—" }, optional: { $0.currentPoint != nil }),
]

/// One operation: what it is, its source bytes, and the graphics state
/// before and after it (the web `OpStatePanel`).
struct OpStatePanel: View {
    let model: DocumentModel
    let content: ContentRef
    let summary: ContentSummary
    let index: UInt32
    @State private var state: OpState?
    @State private var o: OpView?
    @State private var source: Data?
    @State private var error: String?

    private static let maxSource: UInt64 = 4096

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                if let error {
                    Text(error).foregroundStyle(.red)
                } else if let s = state {
                    panel(s)
                } else {
                    ProgressView().controlSize(.small)
                }
            }
            .font(.caption)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: model.generation) {
            do {
                async let st = model.backend.opState(model.doc, content, index: index)
                async let ov = model.backend.contentOps(
                    model.doc, content, range: ListRange(offset: UInt64(index), limit: 1))
                state = try await st
                o = try await ov.items.first
                if let o, let si = o.source, Int(si) < summary.sources.count, let id = summary.sources[Int(si)].id,
                    let span = o.span
                {
                    source = try? await model.backend.streamData(
                        model.doc, id: id, decoded: true, offset: span.start,
                        len: UInt32(min(span.end - span.start, Self.maxSource))
                    ).data
                }
            } catch is CancellationError {
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    @ViewBuilder private func panel(_ s: OpState) -> some View {
        HStack(spacing: 6) {
            Text(s.operator).font(.callout.monospaced().bold())
                .foregroundStyle(Category.color(o?.category ?? "unknown"))
            Text("#\(index) · \(s.description)").foregroundStyle(.secondary)
        }
        if let o {
            ForEach(Array(o.issues.enumerated()), id: \.offset) { _, i in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Badge(i.code, color: i.severity.color)
                    Text(i.message)
                }
            }
            if let t = o.text {
                Text("Text: \(Text("“\(t)”").foregroundStyle(.orange))")
            }
            if let r = o.resource {
                HStack(spacing: 6) {
                    Text("Resource:")
                    ObjLink(model: model, id: r)
                    if o.operator == "Do", o.element?.kind == "form" {
                        LinkButton("its operations →") { model.nav.open(.content(.stream(id: r), op: nil)) }
                    }
                }
            }
            if let e = o.element {
                Text(
                    "Paints: \(e.kind) \(rectText(e.bbox))\(Text(e.hidden ? " (hidden: outside the clip, or invisible text)" : "").foregroundStyle(.secondary))"
                )
            }
            if let source {
                ScrollView {
                    Text(bytesAsText(source) + ((o.span.map { $0.end - $0.start } ?? 0) > Self.maxSource ? " …" : ""))
                        .font(.system(size: 11, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(4)
                }
                .frame(maxHeight: 120)
                .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 4))
            }
            if let si = o.source, Int(si) < summary.sources.count, let span = o.span {
                HStack(spacing: 4) {
                    Text(summary.sources.count > 1 ? "Stream [\(si)]" : "Stream")
                    if let id = summary.sources[Int(si)].id { ObjLink(model: model, id: id) }
                    Text("bytes \(span.start.formatted())–\(span.end.formatted()) (decoded)")
                }
                .foregroundStyle(.secondary)
            }
        }
        let shown = fields.filter { f in f.optional.map { $0(s.before) || $0(s.after) } ?? true }
        Grid(alignment: .topLeading, horizontalSpacing: 6, verticalSpacing: 1) {
            GridRow {
                Text("")
                Text("Before").bold()
                Text("After").bold()
            }
            ForEach(shown, id: \.label) { f in
                let changed = f.text(s.before) != f.text(s.after)
                GridRow {
                    Text(f.label).foregroundStyle(.secondary).frame(width: 80, alignment: .leading)
                    value(f, s.before)
                    if changed { value(f, s.after) } else { Text("=").foregroundStyle(.secondary) }
                }
                .background(changed ? Color.yellow.opacity(0.18) : .clear)
            }
        }
        .textSelection(.enabled)
        if let f = s.after.font ?? s.before.font, let id = f.id {
            HStack(spacing: 6) {
                Text("Font").foregroundStyle(.secondary)
                ObjLink(model: model, id: id)
                LinkButton("glyphs") { model.nav.open(.font(id)) }
            }
        }
    }

    private func value(_ f: Field, _ g: GraphicsState) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            if let c = f.color?(g), let sw = swatch(c) {
                Rectangle().fill(sw).frame(width: 10, height: 10).border(.separator)
            }
            Text(f.text(g)).monospaced()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
