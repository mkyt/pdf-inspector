import Foundation
import Observation

/// One open document as its window sees it: the summary, where the window
/// is, the jobs it started, and editing. Kept current from the session's
/// events; views reload what they read when `generation` changes.
@MainActor @Observable
public final class DocumentModel {
    public let backend: Backend
    public let doc: DocId
    public private(set) var summary: DocSummary
    /// Bumped when the document changes (edit, undo, redo, save): everything
    /// read from it before is stale.
    public private(set) var generation = 0
    public let nav = Navigation()
    /// Jobs started from this window, by id, as their events report them.
    public private(set) var jobs: [JobId: JobInfo] = [:]
    /// Reports about edits and saves, newest last.
    public private(set) var messages: [Message] = []
    @ObservationIgnored private var nextMessage = 1
    @ObservationIgnored private var events: Task<Void, Never>?

    public init(backend: Backend = .shared, summary: DocSummary) {
        self.backend = backend
        self.doc = summary.info.doc
        self.summary = summary
        let stream = backend.events()
        events = Task { [weak self] in
            for await ev in stream {
                self?.handle(ev)
            }
        }
    }

    /// Stop following the session (the window closed).
    public func stop() {
        events?.cancel()
        events = nil
    }

    public var edit: EditStatus { summary.edit }

    private func handle(_ ev: Event) {
        switch ev {
        case .docChanged(let doc, _, _) where doc == self.doc:
            // Jobs ran on the document as it was.
            jobs = [:]
            generation += 1
            Task { await refresh() }
        case .jobProgress(let job, _, let done, let total):
            guard let j = jobs[job] else { return }
            jobs[job] = JobInfo(
                job: j.job, doc: j.doc, kind: j.kind, state: j.state, done: done, total: total,
                result: j.result, error: j.error)
        case .jobDone(let info) where jobs[info.job] != nil:
            jobs[info.job] = info
            // Checking adds diagnostics, the inventory fills fonts and images.
            generation += 1
            Task { await refresh() }
        default:
            break
        }
    }

    public func refresh() async {
        if let s = try? await backend.summary(doc) {
            summary = s
        }
    }

    // MARK: Jobs

    @discardableResult
    public func startJob(_ kind: JobKind) async throws -> JobInfo {
        let j = try await backend.startJob(doc, kind)
        jobs[j.job] = j
        // A short job may have finished before its start was answered.
        if j.state == .running, let now = try? await backend.jobStatus(j.job),
            now.state != .running
        {
            jobs[j.job] = now
            generation += 1
            await refresh()
        }
        return jobs[j.job] ?? j
    }

    public func cancelJob(_ job: JobId) async {
        if let j = try? await backend.cancelJob(job) {
            jobs[job] = j
        }
    }

    /// The newest job of this window matching `kind`.
    public func latestJob(where matches: (JobKind) -> Bool) -> JobInfo? {
        jobs.values.filter { matches($0.kind) }.max { $0.job < $1.job }
    }

    // MARK: Editing

    /// Apply commands as one undoable step; reports the notes and the
    /// schema problems of the changed objects.
    @discardableResult
    public func apply(_ label: String, _ commands: [EditCommand]) async throws -> ApplyResult {
        let r = try await backend.apply(doc, label: label, commands: commands)
        summary = try await backend.summary(doc)
        if !r.notes.isEmpty { report(.info, label, r.notes) }
        if !r.issues.isEmpty {
            var lines = r.issues.prefix(Self.issueLines).map { i in
                (i.obj.map { "\($0.num) \($0.generation): " } ?? "") + (i.path.map { "\($0): " } ?? "")
                    + i.message
            }
            if r.issues.count > Self.issueLines { lines.append("… \(r.issues.count - Self.issueLines) more") }
            report(.warning, "\(label): the changed objects have schema problems", lines)
        }
        return r
    }

    private static let issueLines = 6

    public func undo() async throws {
        _ = try await backend.undo(doc)
        summary = try await backend.summary(doc)
    }

    public func redo() async throws {
        _ = try await backend.redo(doc)
        summary = try await backend.summary(doc)
    }

    // MARK: Messages

    /// Something to tell about an edit or a save. Errors stay until
    /// dismissed, the others fade.
    public struct Message: Identifiable, Sendable {
        public enum Kind: Sendable { case info, warning, error }
        public let id: Int
        public let kind: Kind
        public let title: String
        public let lines: [String]
    }

    public func report(_ kind: Message.Kind, _ title: String, _ lines: [String] = []) {
        let id = nextMessage
        nextMessage += 1
        messages = messages.suffix(3) + [Message(id: id, kind: kind, title: title, lines: lines)]
        guard kind != .error else { return }
        Task {
            try? await Task.sleep(for: .seconds(8))
            self.dismiss(id)
        }
    }

    public func dismiss(_ id: Int) {
        messages.removeAll { $0.id == id }
    }

    /// The document was saved and reopened from the file (fresh history).
    public func reloaded(_ s: DocSummary) {
        summary = s
    }
}
