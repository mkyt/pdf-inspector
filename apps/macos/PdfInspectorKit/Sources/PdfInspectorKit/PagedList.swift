import Foundation
import Observation

/// One slice of a list, as every ranged request returns it.
public protocol ListPage: Sendable {
    associatedtype Item: Sendable
    var total: UInt64 { get }
    var offset: UInt64 { get }
    var items: [Item] { get }
}

extension ObjectSummaryPage: ListPage {}
extension NodeViewPage: ListPage {}
extension XrefRowPage: ListPage {}
extension DiagnosticInfoPage: ListPage {}
extension PageSummaryPage: ListPage {}
extension AnnotInfoPage: ListPage {}
extension OutlineItemPage: ListPage {}
extension FontInfoPage: ListPage {}
extension ImageInfoPage: ListPage {}
extension ReferrerPage: ListPage {}
extension ObjRefPage: ListPage {}
extension OpViewPage: ListPage {}
extension ContentIssuePage: ListPage {}
extension ElementViewPage: ListPage {}
extension GlyphViewPage: ListPage {}
extension ProgramGlyphPage: ListPage {}
extension XmpNodePage: ListPage {}
extension FontUsePage: ListPage {}

/// A long list fetched page by page as rows come into view (like the web
/// `PagedList`). Views call `need(_:)` when a row appears and read `item(_:)`.
@MainActor @Observable
public final class PagedList<P: ListPage> {
    public static var pageSize: Int { 200 }

    /// Length of the whole list: as given, then as the backend reports it.
    public private(set) var total: Int
    public private(set) var error: String?
    private var pages: [Int: P] = [:]
    @ObservationIgnored private var loading: [Int: Task<Void, Never>] = [:]
    @ObservationIgnored private let fetch: @Sendable (ListRange) async throws -> P

    public init(total: Int = 0, fetch: @escaping @Sendable (ListRange) async throws -> P) {
        self.total = total
        self.fetch = fetch
    }

    public func item(_ index: Int) -> P.Item? {
        guard let page = pages[index / Self.pageSize] else { return nil }
        let k = index - Int(page.offset)
        return k >= 0 && k < page.items.count ? page.items[k] : nil
    }

    /// Fetch the page holding row `index` unless it is there or on its way.
    public func need(_ index: Int) {
        let p = index / Self.pageSize
        guard pages[p] == nil, loading[p] == nil else { return }
        let range = ListRange(offset: UInt64(p * Self.pageSize), limit: UInt32(Self.pageSize))
        loading[p] = Task { [fetch] in
            do {
                let page = try await fetch(range)
                self.pages[p] = page
                self.total = Int(page.total)
                self.error = nil
            } catch is CancellationError {
            } catch {
                self.error = error.localizedDescription
            }
            self.loading[p] = nil
        }
    }

    /// Drop what was fetched (the document changed); rows fetch again.
    public func reset(total: Int? = nil) {
        loading.values.forEach { $0.cancel() }
        loading = [:]
        pages = [:]
        error = nil
        if let total { self.total = total }
    }
}
