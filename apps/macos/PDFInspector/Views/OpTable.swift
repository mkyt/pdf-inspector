import AppKit
import PdfInspectorKit
import SwiftUI

/// The rows of the operation list in an NSTableView, which asks only for
/// the rows on screen. (SwiftUI's `List` diffs every row on each update:
/// seconds for a selection among 50,000 operations.) Rows are `OpRow`s;
/// `selection` holds operations, the keyboard and the menu go to `actions`.
struct OpTable: NSViewRepresentable {
    let list: PagedList<OpViewPage>
    /// `list.version`, read in the owner's body so that it redraws.
    let version: Int
    let layout: OpLayout
    let folds: Folds
    /// The focused operation (from the selection or elsewhere).
    let op: UInt32?
    @Binding var selection: Set<Int>
    let actions: Actions

    @MainActor
    struct Actions {
        /// Fold or unfold the block at an operation (to its last operation).
        var toggle: (UInt32, UInt32?) -> Void
        /// Return or a double click.
        var edit: () -> Void
        /// A typed character without ⌘; true when handled.
        var character: (_ c: String, _ shift: Bool) -> Bool
        /// ⌥↑ / ⌥↓.
        var move: (_ down: Bool) -> Void
        /// ← / →; true when it folded or unfolded.
        var fold: (_ right: Bool) -> Bool
        var delete: () -> Void
        var copy: () -> Void
        var cut: () -> Void
        var paste: (String) -> Void
        /// The context menu: titles and actions, nil for a separator.
        var menu: [(title: String, action: () -> Void)?]
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = Table()
        table.coordinator = context.coordinator
        let column = NSTableColumn(identifier: .init("op"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.rowHeight = 20
        table.intercellSpacing = .zero
        table.usesAutomaticRowHeights = false
        table.allowsMultipleSelection = true
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.target = context.coordinator
        table.doubleAction = #selector(Coordinator.doubleClick)
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        context.coordinator.table = table
        context.coordinator.shown = self
        table.reloadData()
        context.coordinator.syncSelection()
        if let op {
            // First shown: the focused operation in the middle.
            let row = layout.row(Int(op))
            DispatchQueue.main.async {
                let r = table.rect(ofRow: row)
                let h = scroll.contentView.bounds.height
                table.scroll(NSPoint(x: 0, y: max(0, r.midY - h / 2)))
            }
        }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let c = context.coordinator
        c.parent = self
        c.update()
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var parent: OpTable
        /// What the table shows now.
        var shown: OpTable?
        weak var table: Table?
        /// Selecting rows from `selection`: not a change by the user.
        private var syncing = false

        init(_ parent: OpTable) {
            self.parent = parent
        }

        func update() {
            guard let table, let old = shown else { return }
            let p = parent
            shown = p
            Perf.tick("OpTable.update")
            if old.layout != p.layout || old.folds != p.folds {
                Perf.tick("OpTable.reloadData")
                table.reloadData()
            } else {
                var rows = IndexSet()
                if old.version != p.version {
                    rows.insert(integersIn: Range(table.rows(in: table.visibleRect)) ?? 0..<0)
                }
                if old.op != p.op {
                    for o in [old.op, p.op].compactMap({ $0 }) { rows.insert(p.layout.row(Int(o))) }
                }
                rows = rows.filteredIndexSet { $0 < table.numberOfRows }
                if !rows.isEmpty {
                    Perf.tick("OpTable.reloadRows")
                    table.reloadData(forRowIndexes: rows, columnIndexes: [0])
                }
            }
            syncSelection()
            if old.op != p.op, let op = p.op {
                table.scrollRowToVisible(p.layout.row(Int(op)))
            }
        }

        /// Select the rows of `selection` (a folded block by its opener).
        func syncSelection() {
            guard let table, let p = shown else { return }
            let rows = IndexSet(p.selection.map { p.layout.row($0) }.filter { $0 < p.layout.rows })
            guard rows != table.selectedRowIndexes else { return }
            syncing = true
            table.selectRowIndexes(rows, byExtendingSelection: false)
            syncing = false
        }

        func numberOfRows(in tableView: NSTableView) -> Int { shown?.layout.rows ?? 0 }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let p = shown else { return nil }
            Perf.tick("OpTable.viewFor")
            let i = p.layout.op(row)
            p.list.need(i)
            let o = p.list.item(i)
            let toggle = p.actions.toggle
            let content = OpCell(
                o: o, active: p.op == UInt32(i), folded: p.folds[UInt32(i)],
                onFold: { toggle(UInt32(i), o?.end) })
            let id = NSUserInterfaceItemIdentifier("OpCell")
            if let cell = tableView.makeView(withIdentifier: id, owner: nil) as? NSHostingView<OpCell> {
                cell.rootView = content
                return cell
            }
            let cell = NSHostingView(rootView: content)
            cell.identifier = id
            cell.sizingOptions = []
            return cell
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !syncing, let table, let p = shown else { return }
            let ops = Set(table.selectedRowIndexes.map { p.layout.op($0) })
            if ops != p.selection { p.$selection.wrappedValue = ops }
        }

        @objc func doubleClick() {
            guard let table, table.clickedRow >= 0 else { return }
            parent.actions.edit()
        }

        func menu(at row: Int) -> NSMenu? {
            guard let table, row >= 0 else { return nil }
            if !table.selectedRowIndexes.contains(row) {
                table.selectRowIndexes([row], byExtendingSelection: false)
            }
            let menu = NSMenu()
            for item in parent.actions.menu {
                if let item {
                    menu.addItem(MenuAction.item(item.title, item.action))
                } else {
                    menu.addItem(.separator())
                }
            }
            return menu
        }
    }

    /// The table, with the keys and commands of the list.
    final class Table: NSTableView {
        weak var coordinator: Coordinator?
        private var actions: Actions? { coordinator?.parent.actions }

        override func keyDown(with event: NSEvent) {
            guard let a = actions else { return super.keyDown(with: event) }
            let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            switch event.keyCode {
            case 36, 76:  // Return, Enter
                a.edit()
                return
            case 51, 117:  // Delete, Forward Delete
                a.delete()
                return
            case 125, 126:  // ↓ / ↑, with ⌥ moving the selection
                if mods.contains(.option) {
                    a.move(event.keyCode == 125)
                    return
                }
            case 123, 124:  // ← / →
                if a.fold(event.keyCode == 124) { return }
            default:
                if !mods.contains(.command), let c = event.charactersIgnoringModifiers, !c.isEmpty,
                    a.character(c, mods.contains(.shift))
                {
                    return
                }
            }
            super.keyDown(with: event)
        }

        override func menu(for event: NSEvent) -> NSMenu? {
            coordinator?.menu(at: row(at: convert(event.locationInWindow, from: nil)))
        }

        @objc func copy(_ sender: Any?) { actions?.copy() }
        @objc func cut(_ sender: Any?) { actions?.cut() }
        @objc func paste(_ sender: Any?) {
            if let s = NSPasteboard.general.string(forType: .string), !s.isEmpty { actions?.paste(s) }
        }
        @objc func delete(_ sender: Any?) { actions?.delete() }

        override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
            switch item.action {
            case #selector(copy(_:)), #selector(cut(_:)), #selector(delete(_:)): selectedRowIndexes.count > 0
            case #selector(paste(_:)): NSPasteboard.general.string(forType: .string) != nil
            default: super.validateUserInterfaceItem(item)
            }
        }
    }
}

/// A row of the table: the operation, or a spinner while its page loads.
struct OpCell: View {
    let o: OpView?
    let active: Bool
    let folded: UInt32?
    let onFold: () -> Void

    var body: some View {
        let _ = Perf.tick("OpCell")
        Group {
            if let o {
                OpRow(o: o, active: active, folded: folded, onFold: onFold)
            } else {
                ProgressView().controlSize(.mini)
            }
        }
        .padding(.horizontal, 6)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }
}

/// The target of a menu item, running a closure (kept by the item).
private final class MenuAction: NSObject {
    private let run: () -> Void

    init(_ run: @escaping () -> Void) {
        self.run = run
    }

    @objc func fire() { run() }

    static func item(_ title: String, _ run: @escaping () -> Void) -> NSMenuItem {
        let target = MenuAction(run)
        let item = NSMenuItem(title: title, action: #selector(fire), keyEquivalent: "")
        item.target = target
        item.representedObject = target
        return item
    }
}
