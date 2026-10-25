import Foundation

/// Bytes of a file or a stream, with where they are.
public struct Bytes: Sendable {
    public let info: BytesInfo
    public let data: Data
}

/// Typed wrappers over the protocol, one per request (like
/// `apps/web/src/api.ts`).
extension Backend {
    private func call<T>(
        _ request: Request, _ pick: (ResponseBody) -> T?
    ) async throws -> T {
        try await callWithBlobs(request, pick).0
    }

    private func callWithBlobs<T>(
        _ request: Request, _ pick: (ResponseBody) -> T?
    ) async throws -> (T, [Data]) {
        let r = try await send(request)
        guard let v = pick(r.body) else { throw UnexpectedResponse(body: r.body) }
        return (v, r.blobs)
    }

    private func part(_ blobs: [Data], _ i: UInt32) -> Data {
        Int(i) < blobs.count ? blobs[Int(i)] : Data()
    }

    // MARK: Documents

    public func hello() async throws -> HelloInfo {
        try await call(.hello(protocol: protocolVersion())) {
            if case .hello(let v) = $0 { v } else { nil }
        }
    }

    public func open(path: String, password: String? = nil, recover: Bool = false)
        async throws -> DocSummary
    {
        try await call(.open(source: .path(path: path), password: password, recover: recover)) {
            if case .doc(let v) = $0 { v } else { nil }
        }
    }

    public func docs() async throws -> [DocInfo] {
        try await call(.docs) { if case .docs(let v) = $0 { v } else { nil } }
    }

    public func close(_ doc: DocId) async throws {
        _ = try await send(.close(doc: doc))
    }

    public func summary(_ doc: DocId) async throws -> DocSummary {
        try await call(.summary(doc: doc)) { if case .doc(let v) = $0 { v } else { nil } }
    }

    public func document(_ doc: DocId) async throws -> DocumentView {
        try await call(.document(doc: doc)) { if case .document(let v) = $0 { v } else { nil } }
    }

    public func xmp(_ doc: DocId, range: ListRange) async throws -> XmpView {
        try await call(.xmp(doc: doc, range: range)) { if case .xmp(let v) = $0 { v } else { nil } }
    }

    // MARK: Objects

    public func objects(_ doc: DocId, range: ListRange, filter: ObjectFilter? = nil)
        async throws -> ObjectSummaryPage
    {
        try await call(.objects(doc: doc, range: range, filter: filter)) {
            if case .objects(let v) = $0 { v } else { nil }
        }
    }

    public func objectClasses(_ doc: DocId) async throws -> [ObjectClass] {
        try await call(.objectClasses(doc: doc)) {
            if case .objectClasses(let v) = $0 { v } else { nil }
        }
    }

    public func object(_ doc: DocId, id: ObjRef) async throws -> ObjectView {
        try await call(.object(doc: doc, id: id)) { if case .object(let v) = $0 { v } else { nil } }
    }

    public func children(_ doc: DocId, path: NodePath, range: ListRange) async throws
        -> NodeViewPage
    {
        try await call(.children(doc: doc, path: path, range: range)) {
            if case .children(let v) = $0 { v } else { nil }
        }
    }

    public func xref(_ doc: DocId, revision: UInt32?, range: ListRange) async throws
        -> XrefRowPage
    {
        try await call(.xref(doc: doc, revision: revision, range: range)) {
            if case .xref(let v) = $0 { v } else { nil }
        }
    }

    public func diagnostics(_ doc: DocId, range: ListRange) async throws -> DiagnosticInfoPage {
        try await call(.diagnostics(doc: doc, range: range)) {
            if case .diagnostics(let v) = $0 { v } else { nil }
        }
    }

    public func schema(_ doc: DocId, root: NodeRoot) async throws -> ObjectSchema {
        try await call(.schema(doc: doc, root: root)) {
            if case .schema(let v) = $0 { v } else { nil }
        }
    }

    public func referrers(_ doc: DocId, id: ObjRef, range: ListRange) async throws
        -> ReferrerPage
    {
        try await call(.referrers(doc: doc, id: id, range: range)) {
            if case .referrers(let v) = $0 { v } else { nil }
        }
    }

    public func refPath(_ doc: DocId, id: ObjRef) async throws -> RefPath {
        try await call(.refPath(doc: doc, id: id)) {
            if case .refPath(let v) = $0 { v } else { nil }
        }
    }

    public func unreferenced(_ doc: DocId, range: ListRange) async throws -> ObjRefPage {
        try await call(.unreferenced(doc: doc, range: range)) {
            if case .objRefs(let v) = $0 { v } else { nil }
        }
    }

    public func rawBytes(_ doc: DocId, offset: UInt64, len: UInt32) async throws -> Bytes {
        let (info, blobs) = try await callWithBlobs(.rawBytes(doc: doc, offset: offset, len: len)) {
            if case .bytes(let v) = $0 { v } else { nil }
        }
        return Bytes(info: info, data: part(blobs, info.part))
    }

    public func streamData(
        _ doc: DocId, id: ObjRef, decoded: Bool, offset: UInt64, len: UInt32
    ) async throws -> Bytes {
        let (info, blobs) = try await callWithBlobs(
            .streamData(doc: doc, id: id, decoded: decoded, offset: offset, len: len)
        ) { if case .bytes(let v) = $0 { v } else { nil } }
        return Bytes(info: info, data: part(blobs, info.part))
    }

    /// An image XObject as it paints (RGBA, not premultiplied), reduced by
    /// about `reduce` in both directions.
    public func decodeImage(_ doc: DocId, id: ObjRef, reduce: UInt32 = 1) async throws
        -> (info: DecodedImage, data: Data)
    {
        let (info, blobs) = try await callWithBlobs(.decodeImage(doc: doc, id: id, reduce: reduce)) {
            if case .decodedImage(let v) = $0 { v } else { nil }
        }
        return (info, info.part.map { part(blobs, $0) } ?? Data())
    }

    // MARK: Pages and the document model

    public func pages(_ doc: DocId, range: ListRange) async throws -> PageSummaryPage {
        try await call(.pages(doc: doc, range: range)) { if case .pages(let v) = $0 { v } else { nil } }
    }

    public func page(_ doc: DocId, index: UInt32) async throws -> PageDetail {
        try await call(.page(doc: doc, index: index)) {
            if case .pageDetail(let v) = $0 { v } else { nil }
        }
    }

    public func pageIndex(_ doc: DocId, id: ObjRef) async throws -> UInt32? {
        try await call(.pageIndex(doc: doc, id: id)) {
            if case .pageIndex(let v) = $0 { .some(v) } else { nil }
        }
    }

    public func annotations(_ doc: DocId, page: UInt32, range: ListRange) async throws
        -> AnnotInfoPage
    {
        try await call(.annotations(doc: doc, page: page, range: range)) {
            if case .annotations(let v) = $0 { v } else { nil }
        }
    }

    public func outline(_ doc: DocId, parent: ObjRef?, range: ListRange) async throws
        -> OutlineItemPage
    {
        try await call(.outline(doc: doc, parent: parent, range: range)) {
            if case .outline(let v) = $0 { v } else { nil }
        }
    }

    public func fonts(_ doc: DocId, range: ListRange) async throws -> FontInfoPage {
        try await call(.fonts(doc: doc, range: range)) { if case .fonts(let v) = $0 { v } else { nil } }
    }

    public func images(_ doc: DocId, range: ListRange) async throws -> ImageInfoPage {
        try await call(.images(doc: doc, range: range)) {
            if case .images(let v) = $0 { v } else { nil }
        }
    }

    // MARK: Contents

    public func content(_ doc: DocId, _ content: ContentRef) async throws -> ContentSummary {
        try await call(.content(doc: doc, content: content)) {
            if case .content(let v) = $0 { v } else { nil }
        }
    }

    public func contentOps(_ doc: DocId, _ content: ContentRef, range: ListRange) async throws
        -> OpViewPage
    {
        try await call(.contentOps(doc: doc, content: content, range: range)) {
            if case .ops(let v) = $0 { v } else { nil }
        }
    }

    public func contentIssues(_ doc: DocId, _ content: ContentRef, range: ListRange)
        async throws -> ContentIssuePage
    {
        try await call(.contentIssues(doc: doc, content: content, range: range)) {
            if case .contentIssues(let v) = $0 { v } else { nil }
        }
    }

    public func contentText(_ doc: DocId, _ content: ContentRef, range: ListRange) async throws
        -> OpText
    {
        try await call(.contentText(doc: doc, content: content, range: range)) {
            if case .opText(let v) = $0 { v } else { nil }
        }
    }

    public func contentBlocks(_ doc: DocId, _ content: ContentRef, depth: UInt32) async throws
        -> (blocks: [BlockSpan], truncated: Bool)
    {
        try await call(.contentBlocks(doc: doc, content: content, depth: depth)) {
            if case .blocks(let b, let t) = $0 { (b, t) } else { nil }
        }
    }

    public func contentResources(_ doc: DocId, _ content: ContentRef) async throws
        -> (items: [ResourceItem], truncated: Bool)
    {
        try await call(.contentResources(doc: doc, content: content)) {
            if case .resources(let i, let t) = $0 { (i, t) } else { nil }
        }
    }

    public func contentElements(_ doc: DocId, _ content: ContentRef, range: ListRange)
        async throws -> ElementViewPage
    {
        try await call(.contentElements(doc: doc, content: content, range: range)) {
            if case .elements(let v) = $0 { v } else { nil }
        }
    }

    public func opState(_ doc: DocId, _ content: ContentRef, index: UInt32) async throws
        -> OpState
    {
        try await call(.opState(doc: doc, content: content, index: index)) {
            if case .opState(let v) = $0 { v } else { nil }
        }
    }

    /// Render a content (or a tile of it) at `scale` pixels per point.
    public func render(
        _ doc: DocId, _ content: ContentRef, scale: Double, tile: PixelRect? = nil,
        until: UInt32? = nil, layer: RenderLayer = .content
    ) async throws -> Bitmap {
        let (info, blobs) = try await callWithBlobs(
            .render(
                doc: doc, content: content, scale: scale, tile: tile, until: until, layer: layer)
        ) { if case .bitmap(let v) = $0 { v } else { nil } }
        return Bitmap(info: info, data: part(blobs, info.part))
    }

    public func hitTest(
        _ doc: DocId, _ content: ContentRef, x: Double, y: Double, radius: Double,
        until: UInt32? = nil
    ) async throws -> [UInt32] {
        try await call(
            .hitTest(doc: doc, content: content, x: x, y: y, radius: radius, until: until)
        ) { if case .hits(let v) = $0 { v } else { nil } }
    }

    /// A one-page PDF of what paints in `rect`.
    public func extractRegion(
        _ doc: DocId, _ content: ContentRef, rect: Rect, select: RegionSelect, fit: Bool
    ) async throws -> (info: ExtractInfo, data: Data) {
        let (info, blobs) = try await callWithBlobs(
            .extractRegion(doc: doc, content: content, rect: rect, select: select, fit: fit)
        ) { if case .extract(let v) = $0 { v } else { nil } }
        return (info, part(blobs, info.part))
    }

    /// SVG of a content; with `rect`, only what paints there (picked as
    /// `extractRegion` picks it).
    public func exportSvg(
        _ doc: DocId, _ content: ContentRef, rect: Rect?, select: RegionSelect, fit: Bool,
        text: SvgText
    ) async throws -> (info: SvgInfo, data: Data) {
        let (info, blobs) = try await callWithBlobs(
            .exportSvg(doc: doc, content: content, rect: rect, select: select, fit: fit, text: text)
        ) { if case .svg(let v) = $0 { v } else { nil } }
        return (info, part(blobs, info.part))
    }

    // MARK: Fonts

    public func font(_ doc: DocId, id: ObjRef) async throws -> FontDetail {
        try await call(.font(doc: doc, id: id)) { if case .font(let v) = $0 { v } else { nil } }
    }

    public func glyphs(_ doc: DocId, id: ObjRef, range: ListRange) async throws -> GlyphViewPage {
        try await call(.glyphs(doc: doc, id: id, range: range)) {
            if case .glyphs(let v) = $0 { v } else { nil }
        }
    }

    public func programGlyphs(_ doc: DocId, id: ObjRef, range: ListRange) async throws
        -> ProgramGlyphPage
    {
        try await call(.programGlyphs(doc: doc, id: id, range: range)) {
            if case .programGlyphs(let v) = $0 { v } else { nil }
        }
    }

    public func fontUses(_ doc: DocId, id: ObjRef, range: ListRange) async throws
        -> FontUsesView
    {
        try await call(.fontUses(doc: doc, id: id, range: range)) {
            if case .fontUses(let v) = $0 { v } else { nil }
        }
    }

    // MARK: Jobs

    public func startJob(_ doc: DocId, _ job: JobKind) async throws -> JobInfo {
        try await call(.startJob(doc: doc, job: job)) { if case .job(let v) = $0 { v } else { nil } }
    }

    public func jobStatus(_ job: JobId) async throws -> JobInfo {
        try await call(.jobStatus(job: job)) { if case .job(let v) = $0 { v } else { nil } }
    }

    public func cancelJob(_ job: JobId) async throws -> JobInfo {
        try await call(.cancelJob(job: job)) { if case .job(let v) = $0 { v } else { nil } }
    }

    public func jobDiagnostics(_ job: JobId, range: ListRange) async throws -> DiagnosticInfoPage {
        try await call(.jobDiagnostics(job: job, range: range)) {
            if case .diagnostics(let v) = $0 { v } else { nil }
        }
    }

    // MARK: Editing

    /// A node in PDF syntax, to edit it as text.
    public func nodeSyntax(_ doc: DocId, path: NodePath) async throws
        -> (text: String, truncated: Bool)
    {
        try await call(.nodeSyntax(doc: doc, path: path)) {
            if case .syntax(let text, let truncated) = $0 { (text, truncated) } else { nil }
        }
    }

    /// Keys the schema type of the dictionary at `path` defines.
    public func keys(_ doc: DocId, path: NodePath) async throws
        -> (typeName: String?, keys: [TypeKey])
    {
        try await call(.keys(doc: doc, path: path)) {
            if case .keys(let typeName, let keys) = $0 { (typeName, keys) } else { nil }
        }
    }

    public func apply(_ doc: DocId, label: String, commands: [EditCommand]) async throws
        -> ApplyResult
    {
        try await call(.apply(doc: doc, label: label, commands: commands)) {
            if case .applied(let v) = $0 { v } else { nil }
        }
    }

    public func undo(_ doc: DocId) async throws -> EditStatus {
        try await call(.undo(doc: doc)) { if case .edit(let v) = $0 { v } else { nil } }
    }

    public func redo(_ doc: DocId) async throws -> EditStatus {
        try await call(.redo(doc: doc)) { if case .edit(let v) = $0 { v } else { nil } }
    }
}
