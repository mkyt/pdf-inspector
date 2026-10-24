import Foundation
import Observation

/// What the main pane of a document window shows (`apps/web/src/nav.ts`).
public enum Target: Hashable, Sendable {
    /// Info, what the catalog declares, XMP.
    case document
    /// The merged trailer.
    case trailer
    case object(ObjRef)
    /// A page by 0-based index.
    case page(UInt32)
    /// The operations of a content, with one of them selected.
    case content(ContentRef, op: UInt32?)
    /// A font dictionary: program, encoding and glyphs.
    case font(ObjRef)

    /// Same place, whatever is selected in it.
    public func isSame(as other: Target) -> Bool {
        switch (self, other) {
        case (.content(let a, _), .content(let b, _)): a == b
        default: self == other
        }
    }
}

/// Where a document window is, with back and forward.
@MainActor @Observable
public final class Navigation {
    public private(set) var target: Target = .document
    private var back: [Target] = []
    private var forward: [Target] = []

    public init() {}

    public var canGoBack: Bool { !back.isEmpty }
    public var canGoForward: Bool { !forward.isEmpty }

    /// Go somewhere new: a new history entry (none if already there).
    public func open(_ t: Target) {
        guard !t.isSame(as: target) else {
            target = t
            return
        }
        back.append(target)
        forward.removeAll()
        target = t
    }

    /// Change the current entry in place (the selected operation).
    public func update(_ t: Target) {
        target = t
    }

    public func goBack() {
        guard let t = back.popLast() else { return }
        forward.append(target)
        target = t
    }

    public func goForward() {
        guard let t = forward.popLast() else { return }
        back.append(target)
        target = t
    }
}
