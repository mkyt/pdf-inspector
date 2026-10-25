import Foundation
import os
import PdfiCore

/// Measuring the app: signposts (Instruments' os_signpost / Points of
/// Interest), logs of slow requests and of the main thread being busy, and
/// counters of how often views are evaluated.
///
/// Signposts are always emitted (they cost next to nothing unless recorded).
/// Logging is on with the user default `PerfLog` (`-PerfLog YES` as a launch
/// argument) and read with
/// `log stream --predicate 'subsystem == "com.juzbox.pdf-inspector"' --level debug`.
public enum Perf {
    public static let subsystem = "com.juzbox.pdf-inspector"
    public static let log = Logger(subsystem: subsystem, category: "perf")
    public static let signposter = OSSignposter(subsystem: subsystem, category: .pointsOfInterest)
    /// Requests to the backend, one interval each.
    static let requests = OSSignposter(subsystem: subsystem, category: "backend")

    /// Logging and counting are on.
    public static let enabled = UserDefaults.standard.bool(forKey: "PerfLog")
    /// Logs are also printed to stderr (`-PerfStderr YES`, for benchmarks).
    public static let stderr = UserDefaults.standard.bool(forKey: "PerfStderr")

    /// Log a measurement (when `enabled`).
    public static func info(_ message: @autoclosure () -> String) {
        guard enabled else { return }
        let m = message()
        log.info("\(m, privacy: .public)")
        if stderr { FileHandle.standardError.write(Data(("perf: " + m + "\n").utf8)) }
    }

    /// Log a detail (when `enabled`), at the debug level.
    public static func debug(_ message: @autoclosure () -> String) {
        guard enabled else { return }
        let m = message()
        log.debug("\(m, privacy: .public)")
        if stderr { FileHandle.standardError.write(Data(("perf: " + m + "\n").utf8)) }
    }

    /// A request slower than this is logged as slow.
    public static let slowRequest: Duration = .milliseconds(50)
    /// A main thread busier than this in one run loop turn is logged.
    public static let slowTurn: Duration = .milliseconds(32)

    /// Time `work` as a signpost interval named `name`.
    public static func measure<T>(_ name: StaticString, _ detail: String = "", _ work: () throws -> T) rethrows -> T {
        let state = signposter.beginInterval(name, id: signposter.makeSignpostID(), "\(detail, privacy: .public)")
        defer { signposter.endInterval(name, state) }
        return try work()
    }

    /// Mark a moment, like a user action, on the timeline.
    public static func event(_ name: StaticString, _ detail: String = "") {
        signposter.emitEvent(name, "\(detail, privacy: .public)")
        debug("\(name) \(detail)")
    }

    // MARK: Counters

    nonisolated(unsafe) private static var counts: [String: Int] = [:]
    private static let countsLock = NSLock()

    /// Count one evaluation of `name` (a view's body, say); cheap when off.
    @inlinable
    public static func tick(_ name: @autoclosure () -> String) {
        guard enabled else { return }
        add(name())
    }

    @usableFromInline
    static func add(_ name: String) {
        countsLock.withLock { counts[name, default: 0] += 1 }
    }

    /// The counts since the last call, cleared.
    public static func takeCounts() -> [String: Int] {
        countsLock.withLock {
            defer { counts = [:] }
            return counts
        }
    }

    // MARK: Requests in flight

    nonisolated(unsafe) private static var inflight = 0

    /// Backend requests running now.
    public static var requestsInFlight: Int { countsLock.withLock { inflight } }

    static func requestStarted() { countsLock.withLock { inflight += 1 } }
    static func requestEnded() { countsLock.withLock { inflight -= 1 } }
}

extension Request {
    /// The kind of request, like `contentOps` (no operands: they can be large).
    public var name: String {
        let m = Mirror(reflecting: self)
        return m.children.first?.label ?? String(describing: self)
    }
}

/// Watches the main run loop: how long each turn keeps the thread busy.
/// Turns longer than `Perf.slowTurn` are logged with the last event before
/// them, and `busy` sums the time for benchmarks.
@MainActor
public final class MainThreadMonitor {
    public static let shared = MainThreadMonitor()

    private var observer: CFRunLoopObserver?
    private var turnStart: ContinuousClock.Instant?
    /// Main thread time in turns since the last `reset`.
    public private(set) var busy: Duration = .zero
    /// The longest turn since the last `reset`.
    public private(set) var longest: Duration = .zero
    public private(set) var turns = 0
    /// What was going on, for the log of a slow turn.
    public var label = ""

    public func start() {
        guard observer == nil else { return }
        let o = CFRunLoopObserverCreateWithHandler(
            nil, CFRunLoopActivity.afterWaiting.rawValue | CFRunLoopActivity.beforeWaiting.rawValue, true,
            // After Core Animation's commit (2_000_000), so the turn includes it.
            Int.max
        ) { _, activity in
            MainActor.assumeIsolated {
                MainThreadMonitor.shared.observe(activity)
            }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), o, .commonModes)
        observer = o
        turnStart = .now
    }

    public func reset() {
        busy = .zero
        longest = .zero
        turns = 0
    }

    private func observe(_ activity: CFRunLoopActivity) {
        let now = ContinuousClock.now
        if activity == .afterWaiting {
            turnStart = now
            return
        }
        guard let start = turnStart else { return }
        turnStart = nil
        let d = now - start
        busy += d
        longest = max(longest, d)
        turns += 1
        if d > Perf.slowTurn {
            Perf.signposter.emitEvent("slow main turn", "\(d.ms) ms \(self.label)")
            Perf.info("main thread busy \(d.ms) ms after \(label)")
        }
    }
}

extension Duration {
    /// In milliseconds, rounded to a tenth.
    public var ms: Double {
        let (s, a) = components
        return ((Double(s) * 1000 + Double(a) / 1e15) * 10).rounded() / 10
    }
}
