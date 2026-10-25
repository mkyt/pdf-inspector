import AppKit
import PdfInspectorKit
import SwiftUI

/// Side of a rendered tile, in device pixels.
private let tileSide: UInt32 = 512
/// Click tolerance, in points.
private let hitRadius = 4.0
/// Largest highlight bitmap side; beyond it only the visible part is asked for.
private let maxHighlight: UInt32 = 2048
/// Annotations of a page fetched for the overlay.
private let maxAnnots: UInt32 = 2000
private let zooms = [0.1, 0.17, 0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4, 6, 8, 12, 16]

/// A rendered content, drawn in tiles of the visible area (the web
/// `RenderView`). Clicking selects the topmost operation painting there
/// (again: the one below it); the selected operation is highlighted; with
/// `step`, only the operations up to it are drawn. A page's annotations
/// can be shown over it: their appearance streams and outlines of where
/// they are; a label selects one. In region mode, dragging marks a
/// rectangle to cut out as a PDF.
struct RenderView: View {
    let model: DocumentModel
    let content: ContentRef
    let summary: ContentSummary
    let op: UInt32?
    let step: Bool
    let onPick: (UInt32) -> Void
    @Binding var annot: UInt32?
    @Environment(\.displayScale) private var dpr
    /// Points per point; nil: fit the whole view.
    @State private var zoom: Double?
    @State private var box = CGSize(width: 400, height: 400)
    @State private var visible = CGRect.zero
    @State private var tiles: [TileKey: CGImage] = [:]
    @State private var highlight: (rect: PixelRect, image: CGImage, key: String)?
    @State private var element: ElementView?
    @State private var notes: [RenderNote] = []
    @State private var hint: String?
    @State private var last: (point: CGPoint, ops: [UInt32])?
    @State private var annots: [AnnotInfo] = []
    @State private var annotsTotal: UInt64 = 0
    @State private var showAp = true
    @State private var showOutlines = true
    @State private var picks: [UInt32: AppearancePick] = [:]
    @State private var cutting = false
    @State private var region: CGRect?
    @State private var dragStart: CGPoint?
    @State private var error: String?

    private var v: ViewInfo { summary.view }
    private var page: UInt32? { if case .page(let i) = content { i } else { nil } }
    private var fit: Double {
        guard box.width > 0, box.height > 0, v.width > 0, v.height > 0 else { return 1 }
        return max(0.05, min((box.width - 2) / v.width, (box.height - 2) / v.height))
    }
    private var z: Double { zoom ?? fit }
    private var scale: Double { z * dpr }
    private var fw: UInt32 { UInt32(max(1, (v.width * scale).rounded(.up))) }
    private var fh: UInt32 { UInt32(max(1, (v.height * scale).rounded(.up))) }
    private var until: UInt32? { step ? op : nil }
    private var canvasSize: CGSize { CGSize(width: Double(fw) / dpr, height: Double(fh) / dpr) }
    /// Where the canvas sits in the scroll content: centered when smaller.
    private var inset: CGPoint {
        CGPoint(x: max(0, (box.width - canvasSize.width) / 2), y: max(0, (box.height - canvasSize.height) / 2))
    }

    struct TileKey: Hashable {
        let layer: String
        let scale: Double
        let until: UInt32?
        let x: UInt32
        let y: UInt32
    }

    /// Tiles overlapping the visible area (device pixels).
    private var visibleTiles: [PixelRect] {
        let r = visible.offsetBy(dx: -inset.x, dy: -inset.y)
        guard fw > 0, fh > 0 else { return [] }
        let x0 = UInt32(max(0, (r.minX * dpr) / Double(tileSide)).rounded(.down))
        let y0 = UInt32(max(0, (r.minY * dpr) / Double(tileSide)).rounded(.down))
        let x1 = min(UInt32(max(0, (r.maxX * dpr / Double(tileSide)).rounded(.up))), (fw + tileSide - 1) / tileSide)
        let y1 = min(UInt32(max(0, (r.maxY * dpr / Double(tileSide)).rounded(.up))), (fh + tileSide - 1) / tileSide)
        var out: [PixelRect] = []
        guard x0 < x1, y0 < y1 else { return out }
        for ty in y0..<y1 {
            for tx in x0..<x1 {
                let x = tx * tileSide
                let y = ty * tileSide
                out.append(PixelRect(x: x, y: y, width: min(tileSide, fw - x), height: min(tileSide, fh - y)))
            }
        }
        return out
    }

    private var annotsLayer: RenderLayer { .annots(only: nil, pick: Array(picks.values)) }
    /// The annotation layer's cache name: other picks paint otherwise.
    private var annotsName: String {
        "annots " + picks.sorted { $0.key < $1.key }.map { "\($0.key)/\($0.value.kind)/\($0.value.state ?? "")" }
            .joined(separator: ",")
    }
    private var showsAppearances: Bool {
        showAp && annots.contains { $0.variants.contains { $0.toPage != nil } }
    }

    private struct LoadKey: Equatable {
        let tiles: [PixelRect]
        let scale: Double
        let until: UInt32?
        let annots: String?
        let generation: Int
    }

    var body: some View {
        let _ = Perf.tick("RenderView")
        VStack(alignment: .leading, spacing: 4) {
            toolbar
            ScrollView([.horizontal, .vertical]) {
                canvas
                    .frame(width: canvasSize.width, height: canvasSize.height)
                    .padding(.leading, inset.x)
                    .padding(.top, inset.y)
            }
            .onScrollGeometryChange(for: CGRect.self, of: { $0.visibleRect }) { visible = $1 }
            .onGeometryChange(for: CGSize.self, of: { $0.size }) { box = $0 }
            .background(Color.gray.opacity(0.12))
            .border(.separator)
            footer
        }
        .task(id: LoadKey(
            tiles: visibleTiles, scale: scale, until: until, annots: showsAppearances ? annotsName : nil,
            generation: model.generation)
        ) {
            await loadTiles()
        }
        .task(id: HighlightKey(op: op, generation: model.generation)) {
            element = nil
            guard let op else { return }
            element = try? await model.backend.contentOps(
                model.doc, content, range: ListRange(offset: UInt64(op), limit: 1)
            ).items.first?.element
        }
        .task(id: HighlightLoad(element: element, op: op, scale: scale, until: until, tiles: visibleTiles)) {
            await loadHighlight()
        }
        .task(id: model.generation) {
            guard let page else { return }
            if let a = try? await model.backend.annotations(
                model.doc, page: page, range: ListRange(offset: 0, limit: maxAnnots))
            {
                annots = a.items
                annotsTotal = a.total
            }
        }
        .onChange(of: model.generation) { tiles = [:] }
        .onChange(of: page) { picks = [:] }
    }

    private struct HighlightKey: Equatable {
        let op: UInt32?
        let generation: Int
    }

    private struct HighlightLoad: Equatable {
        let element: ElementView?
        let op: UInt32?
        let scale: Double
        let until: UInt32?
        let tiles: [PixelRect]
    }

    // MARK: Drawing

    private var canvas: some View {
        let selAnnot = annots.first { $0.index == annot }
        return ZStack(alignment: .topLeading) {
            if summary.kind == .page { Color.white } else { Checkerboard() }
            layerTiles("content")
            if showsAppearances { layerTiles(annotsName) }
            if let h = highlight {
                Image(decorative: h.image, scale: dpr)
                    .opacity(0.75)
                    .offset(x: Double(h.rect.x) / dpr, y: Double(h.rect.y) / dpr)
                    .allowsHitTesting(false)
            }
            if let e = element {
                let r = v.toView(e.bbox)
                Rectangle()
                    .strokeBorder(Color.red, style: StrokeStyle(lineWidth: 1, dash: [4, 2]))
                    .frame(width: r.width * z + 2, height: r.height * z + 2)
                    .offset(x: r.minX * z - 1, y: r.minY * z - 1)
                    .allowsHitTesting(false)
            }
            if let region {
                Rectangle()
                    .fill(Color.blue.opacity(0.08))
                    .strokeBorder(Color.blue, style: StrokeStyle(lineWidth: 1, dash: [4, 2]))
                    .frame(width: region.width * z, height: region.height * z)
                    .offset(x: region.minX * z, y: region.minY * z)
                    .allowsHitTesting(false)
            }
            if showOutlines || selAnnot != nil {
                AnnotOverlay(
                    annots: showOutlines ? annots : selAnnot.map { [$0] } ?? [], view: v, zoom: z, selected: annot,
                    picks: picks
                ) { i in annot = i == annot ? nil : i }
            }
        }
        .shadow(color: .black.opacity(0.3), radius: 1.5)
        .contentShape(.rect)
        .onTapGesture(coordinateSpace: .local) { p in
            guard !cutting else { return }
            Task { await click(at: p) }
        }
        .simultaneousGesture(
            DragGesture(minimumDistance: 3, coordinateSpace: .local)
                .onChanged { g in
                    guard cutting else { return }
                    let a = clamp(dragStart ?? g.startLocation)
                    dragStart = a
                    let b = clamp(g.location)
                    region = CGRect(x: min(a.x, b.x), y: min(a.y, b.y), width: abs(b.x - a.x), height: abs(b.y - a.y))
                }
                .onEnded { _ in dragStart = nil }
        )
        .pointerStyle(.rectSelection)
    }

    /// A view point (in points of the view) from a canvas location.
    private func clamp(_ p: CGPoint) -> CGPoint {
        CGPoint(x: max(0, min(v.width, p.x / z)), y: max(0, min(v.height, p.y / z)))
    }

    @ViewBuilder private func layerTiles(_ layer: String) -> some View {
        ForEach(visibleTiles, id: \.self) { t in
            if let img = tileImage(layer, t) {
                Image(decorative: img, scale: dpr)
                    .resizable()
                    .interpolation(.medium)
                    .frame(width: Double(t.width) / dpr, height: Double(t.height) / dpr)
                    .offset(x: Double(t.x) / dpr, y: Double(t.y) / dpr)
                    .allowsHitTesting(false)
            }
        }
    }

    /// The tile at the current parameters, or the same tile drawn up to
    /// another operation while the new one renders.
    private func tileImage(_ layer: String, _ t: PixelRect) -> CGImage? {
        let u = layer == "content" ? until : nil
        if let img = tiles[TileKey(layer: layer, scale: scale, until: u, x: t.x, y: t.y)] { return img }
        return tiles.first { $0.key.layer == layer && $0.key.scale == scale && $0.key.x == t.x && $0.key.y == t.y }?
            .value
    }

    private func loadTiles() async {
        // Debounce scrolling and resizing.
        try? await Task.sleep(for: .milliseconds(30))
        if Task.isCancelled { return }
        var layers: [(String, RenderLayer, UInt32?)] = [("content", .content, until)]
        if showsAppearances { layers.append((annotsName, annotsLayer, nil)) }
        // Forget other scales and an annotation layer with other picks.
        if tiles.count > 256 || tiles.keys.contains(where: { $0.scale != scale }) {
            tiles = tiles.filter { $0.key.scale == scale }
        }
        for (name, layer, u) in layers {
            for t in visibleTiles {
                let key = TileKey(layer: name, scale: scale, until: u, x: t.x, y: t.y)
                if tiles[key] != nil { continue }
                do {
                    let b = try await model.backend.render(
                        model.doc, content, scale: scale, tile: t, until: u, layer: layer)
                    if let img = b.cgImage() {
                        if name == "content" {
                            // Drop the same tile at other steps.
                            tiles = tiles.filter { !($0.key.layer == name && $0.key.x == t.x && $0.key.y == t.y) }
                        }
                        tiles[key] = img
                    }
                    if name == "content", b.info.notes != notes { notes = b.info.notes }
                    error = nil
                } catch is CancellationError {
                    return
                } catch {
                    if Task.isCancelled { return }
                    self.error = error.localizedDescription
                    return
                }
            }
        }
    }

    private func loadHighlight() async {
        guard let e = element, let op else {
            highlight = nil
            return
        }
        let r = v.toView(e.bbox)
        // Device pixels, grown a little for strokes drawn at least 3 px wide.
        var x0 = max(0, Int((r.minX * scale).rounded(.down)) - 4)
        var y0 = max(0, Int((r.minY * scale).rounded(.down)) - 4)
        var x1 = min(Int(fw), Int((r.maxX * scale).rounded(.up)) + 4)
        var y1 = min(Int(fh), Int((r.maxY * scale).rounded(.up)) + 4)
        if x1 - x0 > Int(maxHighlight) || y1 - y0 > Int(maxHighlight) {
            let p = visible.offsetBy(dx: -inset.x, dy: -inset.y)
            x0 = max(x0, Int((p.minX * dpr).rounded(.down)))
            y0 = max(y0, Int((p.minY * dpr).rounded(.down)))
            x1 = min(x1, Int((p.maxX * dpr).rounded(.up)))
            y1 = min(y1, Int((p.maxY * dpr).rounded(.up)))
        }
        guard x1 > x0, y1 > y0 else {
            highlight = nil
            return
        }
        let tile = PixelRect(x: UInt32(x0), y: UInt32(y0), width: UInt32(x1 - x0), height: UInt32(y1 - y0))
        let key = "\(op) \(scale) \(String(describing: until)) \(x0) \(y0) \(x1) \(y1)"
        if highlight?.key == key { return }
        guard
            let b = try? await model.backend.render(
                model.doc, content, scale: scale, tile: tile, until: until, layer: .highlight(ops: [op])),
            let img = b.cgImage()
        else { return }
        highlight = (tile, img, key)
    }

    // MARK: Picking

    private func click(at p: CGPoint) async {
        let x = p.x / z
        let y = p.y / z
        guard
            let ops = try? await model.backend.hitTest(
                model.doc, content, x: x, y: y, radius: hitRadius / z, until: until)
        else { return }
        // The click in the page's default space, for the annotations there.
        let q = v.toContent(CGPoint(x: x, y: y))
        let over =
            showOutlines || showAp
            ? annots.filter { a in
                a.rect.map { q.x >= $0.x0 && q.x <= $0.x1 && q.y >= $0.y0 && q.y <= $0.y1 } ?? false
            }.reversed() : []
        let overText =
            over.isEmpty
            ? ""
            : " Over annotation\(over.count > 1 ? "s" : "") \(over.map { "[\($0.index)] \($0.subtype ?? "?")" }.joined(separator: ", ")) (select one by its label)."
        let again =
            last.map { abs($0.point.x - p.x) <= 3 && abs($0.point.y - p.y) <= 3 && $0.ops == ops } ?? false
        last = (p, ops)
        guard !ops.isEmpty else {
            hint = "Nothing in the content is painted here." + overText
            return
        }
        let k = again && op != nil ? ((ops.firstIndex(of: op!) ?? -1) + 1) % ops.count : 0
        onPick(ops[k])
        let more =
            ops.count > 1
            ? "\(ops.count) operations paint here (\(ops.map { "#\($0)" }.joined(separator: ", ")), topmost first); click again for the next."
            : ""
        let h = (more + overText).trimmingCharacters(in: .whitespaces)
        hint = h.isEmpty ? nil : h
    }

    // MARK: Controls

    private func stepZoom(_ d: Int) {
        let next = d > 0 ? zooms.first { $0 > z + 1e-6 } : zooms.last { $0 < z - 1e-6 }
        zoom = next ?? z
    }

    private var toolbar: some View {
        HStack(spacing: 4) {
            Button("Zoom Out", systemImage: "minus.magnifyingglass") { stepZoom(-1) }
            Text("\(Int((z * 100).rounded()))%").monospacedDigit().frame(width: 44)
            Button("Zoom In", systemImage: "plus.magnifyingglass") { stepZoom(1) }
            Toggle("Fit", systemImage: "arrow.up.left.and.arrow.down.right", isOn: Binding {
                zoom == nil
            } set: { zoom = $0 ? nil : z })
            .toggleStyle(.button)
            .help("Fit")
            Toggle("1:1", isOn: Binding { zoom == 1 } set: { zoom = $0 ? 1 : nil })
                .toggleStyle(.button)
                .help("Actual size (100%)")
            Toggle("Region", isOn: $cutting)
                .toggleStyle(.button)
                .help("Drag a rectangle to cut out the operations painting inside it, as a PDF")
                .padding(.leading, 6)
            if !annots.isEmpty {
                Toggle("Appearances", isOn: $showAp)
                    .toggleStyle(.button)
                    .help("Paint the annotations' appearance streams over the content")
                    .padding(.leading, 6)
                Toggle("Outlines (\(annots.count.formatted()))", isOn: $showOutlines)
                    .toggleStyle(.button)
                    .help("Outline each annotation's /Rect, /QuadPoints and paths")
            }
            Spacer()
            Text("\(Int(v.width.rounded())) × \(Int(v.height.rounded())) pt\(v.rotate != 0 ? " · /Rotate \(v.rotate)" : "")")
                .foregroundStyle(.secondary)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
        .controlSize(.small)
        .font(.caption)
        .onChange(of: cutting) { if !cutting { region = nil } }
    }

    @ViewBuilder private var footer: some View {
        if cutting {
            if let region {
                RegionCut(model: model, content: content, view: v, region: region) { self.region = nil }
            } else {
                Text("Drag over the content to mark the region to cut out.").font(.caption).foregroundStyle(.secondary)
            }
        }
        if let a = annots.first(where: { $0.index == annot }) {
            AnnotCard(
                model: model, a: a, total: annotsTotal, contentOps: summary.ops, pick: picks[a.index],
                onPick: { p in picks[a.index] = p }, onClose: { annot = nil })
        }
        if let hint { Text(hint).font(.caption).foregroundStyle(.secondary) }
        if !notes.isEmpty {
            Text("Not exact yet: \(notes.map(\.message).joined(separator: "; ")).")
                .font(.caption).foregroundStyle(.orange)
        }
        if let error { Text(error).font(.caption).foregroundStyle(.red) }
    }
}

// MARK: Annotations

/// `/C` as a color (gray, RGB or naive CMYK), or nil when absent.
func annotColor(_ c: [Double]?) -> Color? {
    guard let c, !c.isEmpty else { return nil }
    let v = { (x: Double) in min(1, max(0, x)) }
    switch c.count {
    case 1: return Color(white: v(c[0]))
    case 3: return Color(red: v(c[0]), green: v(c[1]), blue: v(c[2]))
    case 4:
        let k = 1 - c[3]
        return Color(red: v((1 - c[0]) * k), green: v((1 - c[1]) * k), blue: v((1 - c[2]) * k))
    default: return nil
    }
}

private let subtypeColor: [String: Color] = [
    "Link": Color(red: 0.11, green: 0.49, blue: 0.84), "Widget": Color(red: 0.44, green: 0.28, blue: 0.91),
    "Popup": .gray, "Text": Color(red: 0.94, green: 0.55, blue: 0), "FreeText": Color(red: 0.05, green: 0.52, blue: 0.6),
]

/// The outline color of an annotation.
private func stroke(_ a: AnnotInfo) -> Color {
    guard a.shown else { return Color(white: 0.68) }
    return annotColor(a.color) ?? subtypeColor[a.subtype ?? ""] ?? Color(red: 0.91, green: 0.35, blue: 0.05)
}

/// Label text over `stroke(a)`: black over light colors.
private func ink(_ a: AnnotInfo) -> Color {
    guard a.shown, let c = a.color, c.count == 1 || c.count == 3 else { return .white }
    let (r, g, b) = c.count == 1 ? (c[0], c[0], c[0]) : (c[0], c[1], c[2])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.6 ? .black : .white
}

/// The appearance painted for an annotation: the one picked, else the normal one.
func shownAppearance(_ a: AnnotInfo, _ pick: AppearancePick?) -> AppearanceInfo? {
    guard let pick else { return a.appearance }
    return a.variants.first { $0.kind == pick.kind && $0.state == pick.state }
}

private func affine(_ m: [Double]) -> CGAffineTransform {
    m.count == 6 ? CGAffineTransform(a: m[0], b: m[1], c: m[2], d: m[3], tx: m[4], ty: m[5]) : .identity
}

/// Where a page's annotations sit over its content: `/Rect` (dashed),
/// `/QuadPoints` (filled), ink / vertex / line paths, and for the selected
/// one its appearance's `/BBox` as put on the page (dotted, with the form's
/// origin and axes). Labels (`[index] Subtype`) select an annotation.
private struct AnnotOverlay: View {
    let annots: [AnnotInfo]
    let view: ViewInfo
    let zoom: Double
    let selected: UInt32?
    let picks: [UInt32: AppearancePick]
    let onSelect: (UInt32) -> Void

    var body: some View {
        let _ = Perf.tick("AnnotOverlay")
        let sel = annots.first { $0.index == selected }
        let ap = sel.flatMap { shownAppearance($0, picks[$0.index]) }
        let toCanvas = view.transform.concatenating(CGAffineTransform(scaleX: zoom, y: zoom))
        ZStack(alignment: .topLeading) {
            Canvas { ctx, _ in
                for a in annots {
                    let c = stroke(a)
                    let on = a.index == selected
                    let w: CGFloat = on ? 2 : 1
                    var g = ctx
                    g.opacity = selected == nil || on ? 1 : 0.45
                    for q in a.quads where q.count >= 8 {
                        var p = Path()
                        for (k, i) in [0, 1, 3, 2].enumerated() {
                            let pt = CGPoint(x: q[2 * i], y: q[2 * i + 1]).applying(toCanvas)
                            if k == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
                        }
                        p.closeSubpath()
                        g.fill(p, with: .color(c.opacity(0.18)))
                        g.stroke(p, with: .color(c), lineWidth: 0.5)
                    }
                    for pts in a.paths {
                        var p = Path()
                        for i in stride(from: 0, to: pts.count - 1, by: 2) {
                            let pt = CGPoint(x: pts[i], y: pts[i + 1]).applying(toCanvas)
                            if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
                        }
                        if a.subtype == "Polygon" { p.closeSubpath() }
                        g.stroke(p, with: .color(c), lineWidth: w)
                    }
                    if let r = a.rect {
                        let p = Path(CGRect(x: r.x0, y: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0))
                            .applying(toCanvas)
                        if on { g.fill(p, with: .color(c.opacity(0.08))) }
                        g.stroke(p, with: .color(c), style: StrokeStyle(lineWidth: w, dash: a.shown ? [4, 3] : [1, 3]))
                    }
                }
                if let ap, let b = ap.bbox, let tp = ap.toPage {
                    let m = affine(tp).concatenating(toCanvas)
                    let red = Color.red
                    ctx.stroke(
                        Path(CGRect(x: b.x0, y: b.y0, width: b.x1 - b.x0, height: b.y1 - b.y0)).applying(m),
                        with: .color(red), style: StrokeStyle(lineWidth: 1, dash: [1, 2]))
                    // The form's origin corner and x axis (a third of the box's width).
                    var x = Path()
                    x.move(to: CGPoint(x: b.x0, y: b.y0).applying(m))
                    x.addLine(to: CGPoint(x: b.x0 + (b.x1 - b.x0) / 3, y: b.y0).applying(m))
                    ctx.stroke(x, with: .color(red), lineWidth: 3)
                    var y = Path()
                    y.move(to: CGPoint(x: b.x0, y: b.y0).applying(m))
                    y.addLine(to: CGPoint(x: b.x0, y: b.y0 + (b.y1 - b.y0) / 3).applying(m))
                    ctx.stroke(y, with: .color(red), lineWidth: 1.5)
                }
            }
            .allowsHitTesting(false)
            ForEach(annots, id: \.index) { a in
                if let rect = a.rect {
                    let r = view.toView(rect)
                    let on = a.index == selected
                    Button {
                        onSelect(a.index)
                    } label: {
                        Text("\(a.index) \(a.subtype ?? "?")")
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(ink(a))
                            .padding(.horizontal, 4)
                            .frame(height: 15)
                            .background(stroke(a), in: .rect(cornerRadius: 2))
                            .overlay(RoundedRectangle(cornerRadius: 2).stroke(on ? Color.black : .clear))
                            .fixedSize()
                    }
                    .buttonStyle(.plain)
                    .opacity(selected == nil || on ? 0.9 : 0.5)
                    .offset(x: r.minX * zoom, y: max(0, r.minY * zoom - 15))
                    .help("[\(a.index)] \(a.subtype ?? "?")\(a.shown ? "" : " (not shown)")\(a.contents.map { ": \($0)" } ?? "")")
                    .zIndex(on ? 2 : 1)
                }
            }
        }
    }
}

private func matrixText(_ m: [Double]) -> String { "[\(m.map(numText).joined(separator: " "))]" }

private func variantText(_ v: AppearanceInfo) -> String { "/\(v.kind)" + (v.state.map { " /\($0)" } ?? "") }

/// The selected annotation: what it is, and how its appearance is put over
/// the content.
struct AnnotCard: View {
    let model: DocumentModel
    let a: AnnotInfo
    /// Annotations on the page.
    let total: UInt64
    /// Operations of the page's content (painted before every annotation).
    let contentOps: UInt64
    let pick: AppearancePick?
    let onPick: (AppearancePick?) -> Void
    let onClose: () -> Void

    private func isNormal(_ v: AppearanceInfo) -> Bool {
        guard let n = a.appearance, n.error == nil else { return false }
        return v.kind == n.kind && v.state == n.state
    }

    var body: some View {
        let _ = Perf.tick("AnnotCard")
        let ap = shownAppearance(a, pick)
        let painted = a.shown && ap != nil && ap?.error == nil && ap?.id != nil && ap?.toPage != nil
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Badge("[\(a.index)] \(a.subtype ?? "?")", color: .accentColor)
                if let id = a.id { ObjLink(model: model, id: id) } else { Text("direct").foregroundStyle(.secondary) }
                ForEach(a.flagNames, id: \.self) { Badge($0, outline: true) }
                Spacer()
                Button("Close", systemImage: "xmark", action: onClose).labelStyle(.iconOnly).buttonStyle(.borderless)
            }
            if a.variants.count > 1 || (a.variants.count == 1 && a.appearance?.error != nil) {
                HStack(spacing: 4) {
                    Text("Appearance:").foregroundStyle(.secondary)
                    ForEach(Array(a.variants.enumerated()), id: \.offset) { _, v in
                        let on = pick.map { v.kind == $0.kind && v.state == $0.state } ?? isNormal(v)
                        Toggle(variantText(v) + (isNormal(v) ? " ·AS" : ""), isOn: Binding {
                            on
                        } set: { _ in
                            onPick(isNormal(v) ? nil : AppearancePick(index: a.index, kind: v.kind, state: v.state))
                        })
                        .toggleStyle(.button)
                        .tint(v.error != nil ? .orange : nil)
                        .help(v.error ?? (isNormal(v) ? "What a viewer shows (chosen by /AS)" : ""))
                    }
                    if pick != nil { Text("(not what a viewer shows at rest)").foregroundStyle(.orange) }
                }
                .controlSize(.small)
            }
            Text(paintedText(ap, painted)).foregroundStyle(.secondary)
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 2) {
                if let e = a.error { row("Error") { Text(e).foregroundStyle(.red) } }
                row("/Rect") { Text(a.rect?.text ?? "—").monospaced() }
                if let c = a.contents { row("/Contents") { Text(c) } }
                if a.author != nil || a.subject != nil { row("/T · /Subj") { Text("\(a.author ?? "—") · \(a.subject ?? "—")") } }
                if let m = a.modified { row("/M") { Text(m) } }
                if a.color != nil || a.opacity != nil || a.border != nil {
                    row("/C · /CA · width") {
                        HStack {
                            if let c = annotColor(a.color) {
                                Rectangle().fill(c).frame(width: 12, height: 12).border(.black.opacity(0.2))
                            }
                            Text(
                                "\(a.color.map(matrixText) ?? "—") · \(a.opacity.map(numText) ?? "—") · \(a.border.map(numText) ?? "—")"
                            )
                            .monospaced()
                        }
                    }
                }
                if !a.quads.isEmpty { row("/QuadPoints") { Text("\(a.quads.count) quadrilateral\(a.quads.count == 1 ? "" : "s")") } }
                if !a.paths.isEmpty {
                    row("Paths") { Text("\(a.paths.count) path\(a.paths.count == 1 ? "" : "s"), \(a.paths.reduce(0) { $0 + $1.count / 2 }) points") }
                }
                if let l = a.link { row("Link") { LinkView(model: model, link: l) } }
                if let p = a.parent ?? a.popup { row(a.parent != nil ? "For" : "/Popup") { ObjLink(model: model, id: p) } }
                row("/AP") { Text(a.appearances.isEmpty ? "none" : a.appearances.map { "/\($0)" }.joined(separator: " ")) }
                if let ap {
                    row(ap.kind == "N" ? "Normal" : ap.kind == "R" ? "Rollover" : "Down") {
                        HStack(spacing: 4) {
                            if let id = ap.id {
                                LinkButton("\(id.objText) — open its operations") {
                                    model.nav.open(.content(.stream(id: id), op: nil))
                                }
                                .monospaced()
                            } else {
                                Text("—")
                            }
                            if !ap.states.isEmpty {
                                Text(
                                    "· \(pick != nil ? "state" : "/AS") \(ap.state.map { "/\($0)" } ?? "missing") of \(ap.states.map { "/\($0)" }.joined(separator: " "))"
                                )
                                .foregroundStyle(.secondary)
                            }
                        }
                    }
                    row("/BBox · /Matrix") { Text("\(ap.bbox?.text ?? "—") · \(matrixText(ap.matrix))").monospaced() }
                    if let tp = ap.toPage {
                        row("Form → page") {
                            Text(
                                "\(Text(matrixText(tp)).monospaced())\(Text(" (/Matrix, then /BBox fit to /Rect; dotted on the page, thick edge = form x axis)").foregroundStyle(.secondary))"
                            )
                        }
                    }
                    if let e = ap.error { row("Problem") { Text(e).foregroundStyle(.red) } }
                }
            }
            .textSelection(.enabled)
        }
        .font(.caption)
        .padding(6)
        .background(.background, in: .rect(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
    }

    private func paintedText(_ ap: AppearanceInfo?, _ painted: Bool) -> String {
        if painted {
            let above = Int(total) - Int(a.index) - 1
            return "Painted over all \(contentOps.formatted()) content operations"
                + (a.index > 0 ? " and annotations [0]–[\(a.index - 1)]" : "")
                + "; \(above) annotation\(above == 1 ? "" : "s") above it."
        }
        if !a.shown { return "Not shown on screen (Hidden / NoView, or Invisible without an appearance)." }
        if let e = ap?.error { return "Not painted: \(e)." }
        return "Not painted: no normal appearance stream (/AP /N); a viewer may make one up."
    }

    private func row(_ k: String, @ViewBuilder _ v: () -> some View) -> some View {
        GridRow {
            Text(k).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
            v()
        }
    }
}

/// Where a link goes; a page of this document opens on click.
struct LinkView: View {
    let model: DocumentModel
    let link: LinkTarget

    var body: some View {
        let _ = Perf.tick("LinkView")
        switch link {
        case .dest(let d) where d.pageIndex != nil:
            LinkButton(d.text) { model.nav.open(.page(d.pageIndex!)) }
        case .uri(let u):
            if let url = URL(string: u) {
                Link(u, destination: url)
            } else {
                Text(u)
            }
        case .dest(let d):
            Text(link.text).foregroundStyle(d.error != nil ? .red : .primary)
        case .action:
            Text(link.text)
        }
    }
}

// MARK: Region cut

/// The region dragged over a rendered content, and cutting it out: a
/// one-page PDF with only the operations painting there.
private struct RegionCut: View {
    let model: DocumentModel
    let content: ContentRef
    let view: ViewInfo
    /// In view points.
    let region: CGRect
    let onClear: () -> Void
    @State private var select: RegionSelect = .inside
    @State private var fit = true
    @State private var busy = false
    @State private var done: ExtractInfo?
    @State private var error: String?

    var body: some View {
        let _ = Perf.tick("RegionCut")
        let rect = view.toContent(region)
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 8) {
                Text("Region \(rect.text) (\(numText(rect.x1 - rect.x0)) × \(numText(rect.y1 - rect.y0)) pt)")
                Picker("Select", selection: $select) {
                    Text("Inside").tag(RegionSelect.inside)
                    Text("Touching").tag(RegionSelect.overlaps)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .help("Which painting operations to keep: those painting wholly inside the region, or also those only touching it")
                Toggle("Crop to content", isOn: $fit).help("Shrink the page to what is kept (within the region)")
                Button("Save PDF…") { Task { await run(rect) } }
                    .disabled(busy)
                    .help("A one-page PDF with only the operations painting here; what they use is copied as it is in the file")
                Button("Clear", systemImage: "xmark", action: onClear).labelStyle(.iconOnly).buttonStyle(.borderless)
            }
            .controlSize(.small)
            if let d = done {
                Text(
                    "Kept \(d.kept.formatted()) of \((d.kept + d.dropped).formatted()) painting operations\(d.forms > 0 ? ", \(d.forms) form XObject\(d.forms > 1 ? "s" : "") cut" : ""); \(d.objects) objects, \(bytesText(d.bytes)); /MediaBox \(d.mediaBox.text)."
                )
                .foregroundStyle(.secondary)
            }
            if let error { Text(error).foregroundStyle(.red) }
        }
        .font(.caption)
        .onChange(of: region) { done = nil }
    }

    private func run(_ rect: Rect) async {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.pdf]
        let stem = (model.summary.info.name as NSString).deletingPathExtension
        let what = switch content {
        case .page(let i): "p\(i + 1)"
        case .stream(let id): "\(id.num)-\(id.generation)"
        }
        panel.nameFieldStringValue = "\(stem)-\(what)-region.pdf"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        busy = true
        defer { busy = false }
        do {
            let r = try await model.backend.extractRegion(model.doc, content, rect: rect, select: select, fit: fit)
            try r.data.write(to: url)
            done = r.info
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}
