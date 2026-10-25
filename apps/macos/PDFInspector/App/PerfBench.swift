import AppKit
import PdfInspectorKit
import SwiftUI

/// A benchmark of the content view, run from the command line:
///
///     PDF\ Inspector.app/Contents/MacOS/PDF\ Inspector -PerfBench select -PerfBenchFile <path> [-PerfLog YES]
///
/// Options: `-PerfBenchSteps n` (30), `-PerfBenchPage i` (0-based),
/// `-PerfBenchPanes schema,bytes,<sidebar tab>`, `-PerfBenchCounts n` (how
/// many view counters to list), `-PerfStderr YES` (the logs on stderr),
/// `-PerfBenchSnapshot <png>` (the window at the end), `-PerfBenchVerbose YES`
/// (every step).
///
/// `select` steps the selected operation from code, `keys` with ↓ in the
/// operation list (as typing does), `scroll` pages down the list; `pages`
/// and `objects` open pages and objects one after another (with the
/// sidebar's lists, `-PerfBenchPanes pages` or `objects`); `bytes` grows a
/// hex dump (`BytesDump`) alone. The file must be readable from the
/// sandbox: under `~/Library/Containers/com.juzbox.pdf-inspector/Data`.
/// Each step waits for the window to settle (no request running, the main
/// thread idle) and reports the time, the main thread's share of it and how
/// many views were evaluated; the summary goes to stdout.
@MainActor
enum PerfBench {
    static func runIfAsked() {
        let d = UserDefaults.standard
        guard let mode = d.string(forKey: "PerfBench") else { return }
        guard let path = d.string(forKey: "PerfBenchFile") else {
            print("PerfBench: no -PerfBenchFile")
            exit(2)
        }
        let steps = max(1, d.integer(forKey: "PerfBenchSteps").nonZero ?? 30)
        let page = UInt32(d.integer(forKey: "PerfBenchPage"))
        let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
        Task {
            do {
                let (doc, _) = try await NSDocumentController.shared.openDocument(withContentsOf: url, display: true)
                guard let model = (doc as? InspectorDocument)?.model else { throw Failure("no model") }
                try await run(mode, model: model, page: page, steps: steps)
                exit(0)
            } catch {
                print("PerfBench failed: \(error)")
                exit(1)
            }
        }
    }

    struct Failure: Error, CustomStringConvertible {
        let description: String
        init(_ d: String) { description = d }
    }

    struct Sample {
        let label: String
        let settle: Duration
        let busy: Duration
        let longest: Duration
        let counts: [String: Int]
    }

    /// Wait until no request has run for a while with the main thread free
    /// (polled every few ms).
    static func settle(timeout: Duration = .seconds(20)) async -> Duration {
        let start = ContinuousClock.now
        var quietSince: ContinuousClock.Instant?
        let quiet = Duration.milliseconds(60)
        var last = start
        while ContinuousClock.now - start < timeout {
            try? await Task.sleep(for: .milliseconds(4))
            let now = ContinuousClock.now
            // A long gap: the main thread was busy (the views may start
            // requests after it, like a debounced render). Not quiet.
            if now - last > .milliseconds(20) { quietSince = nil }
            last = now
            if Perf.requestsInFlight == 0 {
                if let q = quietSince {
                    if now - q >= quiet { return q - start }
                } else {
                    quietSince = now
                }
            } else {
                quietSince = nil
            }
        }
        return timeout
    }

    static func measure(_ label: String, _ action: () -> Void) async -> Sample {
        let mon = MainThreadMonitor.shared
        _ = Perf.takeCounts()
        mon.reset()
        mon.label = label
        Perf.event("bench step", label)
        action()
        let t = await settle()
        return Sample(label: label, settle: t, busy: mon.busy, longest: mon.longest, counts: Perf.takeCounts())
    }

    static func run(_ mode: String, model: DocumentModel, page: UInt32, steps: Int) async throws {
        MainThreadMonitor.shared.start()
        // Panes to show as well: `-PerfBenchPanes schema,bytes,objects` (a
        // sidebar tab by name).
        let panes = (UserDefaults.standard.string(forKey: "PerfBenchPanes") ?? "").split(separator: ",").map(String.init)
        if let layout = (NSApp.windows.lazy.compactMap { $0.windowController as? DocumentWindowController }.first)?.layout {
            for p in panes {
                switch p {
                case "schema": layout.schema = true
                case "bytes": layout.bytes = true
                default: if let t = SidebarTab(rawValue: p) { layout.tab = t }
                }
            }
        }
        _ = await settle()
        if mode == "bytes" {
            try await bytes(steps: steps)
            return
        }
        if mode == "grid" {
            try await grid(steps: steps)
            return
        }
        let content = ContentRef.page(index: page)
        let open = await measure("open content") { model.nav.open(.content(content, op: 0)) }
        report("open content", [open])
        var samples: [Sample] = []
        switch mode {
        case "select":
            for i in 1...steps {
                samples.append(await measure("select #\(i)") { model.nav.update(.content(content, op: UInt32(i))) })
            }
        case "keys":
            guard let window = NSApp.windows.first(where: { $0.isVisible && $0.windowController is DocumentWindowController }),
                let table = findTable(window.contentView)
            else { throw Failure("no operation list") }
            window.makeFirstResponder(table)
            for i in 1...steps {
                samples.append(await measure("↓ #\(i)") { press(down: true, in: window) })
            }
        case "pages":
            for i in 1...steps {
                samples.append(await measure("page \(i)") { model.nav.open(.page(UInt32(i))) })
            }
        case "objects":
            for i in 1...steps {
                samples.append(await measure("object \(i)") { model.nav.open(.object(ObjRef(num: UInt32(i), generation: 0))) })
            }
        case "scroll":
            guard let table = NSApp.windows.lazy.compactMap({ findTable($0.contentView) }).first,
                let clip = table.enclosingScrollView?.contentView
            else { throw Failure("no operation list") }
            for i in 1...steps {
                samples.append(await measure("scroll #\(i)") {
                    let y = clip.bounds.origin.y + clip.bounds.height
                    clip.scroll(to: NSPoint(x: 0, y: y))
                    table.enclosingScrollView?.reflectScrolledClipView(clip)
                })
            }
        default:
            throw Failure("unknown mode \(mode)")
        }
        report(mode, samples)
        if let path = UserDefaults.standard.string(forKey: "PerfBenchSnapshot") { snapshot(to: path) }
    }

    /// The document window as a PNG (to see what was measured).
    private static func snapshot(to path: String) {
        guard let w = NSApp.windows.first(where: { $0.windowController is DocumentWindowController }),
            let v = w.contentView?.superview ?? w.contentView,
            let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds)
        else { return }
        v.cacheDisplay(in: v.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }

    /// `BytesDump` alone in a window, given 256 KiB more each step (as
    /// "Load next" does).
    private static func bytes(steps: Int) async throws {
        let chunk = 256 * 1024
        let state = BytesState()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = NSHostingView(rootView: BytesHost(state: state))
        window.makeKeyAndOrderFront(nil)
        var samples: [Sample] = []
        for i in 1...steps {
            samples.append(await measure("bytes \(i * chunk / 1024) KiB") {
                state.data.append(Data((0..<chunk).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ i) }))
            })
        }
        report("bytes (+\(chunk / 1024) KiB a step, to \(steps * chunk / 1024) KiB)", samples)
        if let path = UserDefaults.standard.string(forKey: "PerfBenchSnapshot"), let v = window.contentView,
            let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds)
        {
            v.cacheDisplay(in: v.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        }
    }

    /// A lazy grid of 65,536 cells like the font's glyphs, picking another
    /// cell each step (`-PerfBenchCells n` for another count).
    private static func grid(steps: Int) async throws {
        let state = GridState()
        state.count = UserDefaults.standard.integer(forKey: "PerfBenchCells").nonZero ?? 65536
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = NSHostingView(rootView: GridHost(state: state))
        window.makeKeyAndOrderFront(nil)
        _ = await settle()
        var samples: [Sample] = []
        for i in 1...steps {
            samples.append(await measure("pick \(i)") { state.picked = i })
        }
        report("grid of \(state.count) (picking)", samples)
    }

    /// The operation list.
    private static func findTable(_ v: NSView?) -> NSTableView? {
        guard let v else { return nil }
        if let t = v as? OpTable.Table { return t }
        return v.subviews.lazy.compactMap(findTable).first
    }

    private static func press(down: Bool, in window: NSWindow) {
        let code: UInt16 = down ? 125 : 126
        let chars = String(UnicodeScalar(UInt16(down ? NSDownArrowFunctionKey : NSUpArrowFunctionKey))!)
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            if let e = NSEvent.keyEvent(
                with: type, location: .zero, modifierFlags: [.numericPad, .function],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
                characters: chars, charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)
            {
                window.sendEvent(e)
            }
        }
    }

    private static func report(_ title: String, _ s: [Sample]) {
        func stats(_ xs: [Duration]) -> String {
            let v = xs.map(\.ms).sorted()
            guard !v.isEmpty else { return "-" }
            let p = { (q: Double) in v[min(v.count - 1, Int(Double(v.count - 1) * q))] }
            return String(format: "median %7.1f  p90 %7.1f  max %7.1f ms", p(0.5), p(0.9), v.last!)
        }
        print("== \(title) (\(s.count) steps)")
        if UserDefaults.standard.bool(forKey: "PerfBenchVerbose") {
            for x in s {
                print(String(format: "  %-24@ settle %7.1f  main %7.1f  longest %7.1f ms", x.label as NSString, x.settle.ms, x.busy.ms, x.longest.ms))
            }
        }
        print("  settle   \(stats(s.map(\.settle)))")
        print("  main     \(stats(s.map(\.busy)))")
        print("  longest  \(stats(s.map(\.longest)))")
        var total: [String: Int] = [:]
        for x in s { total.merge(x.counts, uniquingKeysWith: +) }
        for (k, n) in total.sorted(by: { $0.value > $1.value }).prefix(UserDefaults.standard.integer(forKey: "PerfBenchCounts").nonZero ?? 15) {
            print(String(format: "  %-28@ %8.1f per step", k as NSString, Double(n) / Double(s.count)))
        }
        fflush(stdout)
    }
}

@MainActor @Observable
private final class BytesState {
    var data = Data()
}

@MainActor @Observable
private final class GridState {
    var count = 0
    var picked = 0
}

private struct GridHost: View {
    let state: GridState

    var body: some View {
        // As GlyphGrid: the pick comes in as a property.
        GridCells(count: state.count, picked: state.picked) { state.picked = $0 }
    }
}

private struct GridCells: View {
    let count: Int
    let picked: Int
    let pick: (Int) -> Void

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 76, maximum: 76), spacing: 0)], spacing: 0) {
                ForEach(0..<count, id: \.self) { i in
                    Text("\(i)")
                        .frame(width: 76, height: 92)
                        .background(picked == i ? Color.accentColor.opacity(0.2) : .clear)
                        .onTapGesture { pick(i) }
                }
            }
        }
    }
}

private struct BytesHost: View {
    let state: BytesState

    var body: some View {
        BytesDump(data: state.data)
    }
}

extension Int {
    fileprivate var nonZero: Int? { self == 0 ? nil : self }
}
