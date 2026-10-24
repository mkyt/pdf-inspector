import Foundation
@_exported import PdfiCore

// pdfi-api's aliases (the bindings see the plain integers).
public typealias DocId = UInt32
public typealias JobId = UInt32

/// The Rust session (pdfi-ffi), shared by every document of the app: one
/// document can take pages from another.
///
/// Requests block in Rust, so each runs on a thread of its own, never on
/// the caller's; cancelling the calling task stops renders, hit tests and
/// saves while they run.
public final class Backend: Sendable {
    public static let shared = Backend()

    private let session = PdfSession()

    public init() {}

    /// Run `work` on a new thread. Not GCD: its 512 KiB stacks are too small
    /// for the renderer (Rust's own threads get 2 MiB).
    private func run<T: Sendable>(
        _ work: @escaping @Sendable (PdfSession, CancelToken) throws -> T
    ) async throws -> T {
        try Task.checkCancellation()
        let cancel = CancelToken()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { cont in
                let t = Thread { [session] in
                    cont.resume(with: Result { try work(session, cancel) })
                }
                t.stackSize = 8 << 20
                t.qualityOfService = .userInitiated
                t.start()
            }
        } onCancel: {
            cancel.cancel()
        }
    }

    public func send(_ request: Request) async throws -> Reply {
        try await run { try $0.dispatchCancellable(request: request, cancel: $1) }
    }

    /// Blocking, for callers that must answer synchronously (NSDocument
    /// reading and writing); not for anything that renders.
    public func sendNow(_ request: Request) throws -> Reply {
        try session.dispatch(request: request)
    }

    /// Blocking `save`, for NSDocument's writing.
    public func saveNow(_ doc: DocId, mode: SaveMode, to url: URL) throws -> SaveResult {
        try session.saveTo(doc: doc, mode: mode, path: url.path, cancel: CancelToken())
    }

    /// Every event of the session from now on, until the stream is dropped.
    public func events() -> AsyncStream<Event> {
        AsyncStream { cont in
            let id = session.subscribe(listener: Listener(cont))
            cont.onTermination = { [session] _ in session.unsubscribe(id: id) }
        }
    }

    /// Write `doc` with its edits to a new file at `url`; `reload` it from
    /// where the file ends up.
    public func save(_ doc: DocId, mode: SaveMode, to url: URL) async throws -> SaveResult {
        try await run { try $0.saveTo(doc: doc, mode: mode, path: url.path, cancel: $1) }
    }

    /// Replace `doc` by the file at `url` (after saving it), with a fresh
    /// history.
    public func reload(_ doc: DocId, from url: URL) throws -> DocSummary {
        try session.reload(doc: doc, path: url.path)
    }
}

private final class Listener: EventListener {
    let cont: AsyncStream<Event>.Continuation

    init(_ cont: AsyncStream<Event>.Continuation) {
        self.cont = cont
    }

    func onEvent(event: Event) {
        cont.yield(event)
    }
}

extension PdfiError: LocalizedError {
    public var code: ErrorCode {
        switch self {
        case .Api(let code, _): code
        }
    }

    public var message: String {
        switch self {
        case .Api(_, let message): message
        }
    }

    public var errorDescription: String? { message }
}

/// The backend answered with another kind of response than the request
/// calls for.
public struct UnexpectedResponse: LocalizedError {
    public let body: ResponseBody

    public init(body: ResponseBody) {
        self.body = body
    }

    public var errorDescription: String? { "unexpected response: \(body)" }
}
