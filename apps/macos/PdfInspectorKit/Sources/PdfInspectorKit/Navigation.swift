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

/// An entry picked in the structure tree, shown in the schema panel.
public struct Selection: Equatable, Sendable {
    public let node: NodeView
    /// Location in the object, like `/Pages/Kids[3]`.
    public let path: String

    public init(node: NodeView, path: String) {
        self.node = node
        self.path = path
    }
}

/// Where a document window is, with back and forward.
@MainActor @Observable
public final class Navigation {
    public private(set) var target: Target = .document {
        didSet {
            if !target.isSame(as: oldValue) { selection = nil }
        }
    }
    private var back: [Target] = []
    private var forward: [Target] = []
    public private(set) var selection: Selection?
    /// Bytes of the file highlighted in the hex view.
    public private(set) var hexSpan: ByteSpan?
    /// Bumped when the hex view should be shown (not only scrolled).
    public private(set) var bytesRequest = 0

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

    public func select(_ s: Selection?) {
        selection = s
    }

    /// Highlight a span of the file in the hex view and scroll to it;
    /// `reveal` also shows the hex view when it is hidden.
    public func showBytes(_ s: ByteSpan, reveal: Bool = false) {
        hexSpan = s
        if reveal { bytesRequest += 1 }
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
