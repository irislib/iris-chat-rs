#if os(macOS)
import AppKit
import SwiftUI

/// A conversation has one text range, even though each bubble has its own text view.
/// Keep endpoints by message ID so lazily mounted rows and history prepends do not
/// move the selection. Only message bodies participate, never metadata or controls.
final class IrisDesktopMessageSelection: ObservableObject {
    struct Position: Equatable {
        let id: String
        let offset: Int // NSTextView/NSString UTF-16 insertion offset
    }
    private final class Entry {
        weak var view: IrisMessageTextView?
        var text: NSString
        init(_ view: IrisMessageTextView) {
            self.view = view
            text = (view.string as NSString).copy() as! NSString
        }
    }
    private var entries: [String: Entry] = [:]
    private var ids: [String] = []
    private var chatID = ""
    private(set) var anchor: Position?
    private(set) var extent: Position?

    func update(chatID: String, ids: [String]) {
        if self.chatID != chatID {
            clear()
            if !self.chatID.isEmpty { entries.removeAll() }
        } else if let anchor, let extent, !ids.contains(anchor.id) || !ids.contains(extent.id) {
            clear()
        }
        self.chatID = chatID
        self.ids = ids
        entries = entries.filter { ids.contains($0.key) }
        paint()
    }

    func register(_ view: IrisMessageTextView, id: String) {
        if let old = entries[id], old.text != view.string as NSString, range(for: id) != nil { clear() }
        entries[id] = Entry(view)
        paint(view, id: id)
    }

    func clear() {
        anchor = nil
        extent = nil
        paint()
    }

    func select(from: Position, to: Position) {
        anchor = from
        extent = to
        paint()
    }

    func range(for id: String) -> NSRange? {
        guard let anchor, let extent,
              let a = ids.firstIndex(of: anchor.id), let b = ids.firstIndex(of: extent.id),
              let i = ids.firstIndex(of: id), let text = entries[id]?.text else { return nil }
        let forward = a < b || (a == b && anchor.offset <= extent.offset)
        let start = forward ? anchor : extent
        let end = forward ? extent : anchor
        guard i >= min(a, b), i <= max(a, b) else { return nil }
        let lower = id == start.id ? min(max(0, start.offset), text.length) : 0
        let upper = id == end.id ? min(max(0, end.offset), text.length) : text.length
        guard upper > lower else { return nil }
        // A drag must never copy half an emoji or another composed character.
        return text.rangeOfComposedCharacterSequences(for: NSRange(location: lower, length: upper - lower))
    }

    var selectedText: String {
        ids.compactMap { id -> String? in
            guard let range = range(for: id), let text = entries[id]?.text else { return nil }
            return text.substring(with: range)
        }.joined(separator: "\n")
    }

    private func paint() {
        for (id, entry) in entries {
            if let view = entry.view { paint(view, id: id) }
        }
    }

    private func paint(_ view: IrisMessageTextView, id: String) {
        guard let layout = view.layoutManager else { return }
        let full = NSRange(location: 0, length: (view.string as NSString).length)
        layout.removeTemporaryAttribute(.backgroundColor, forCharacterRange: full)
        layout.removeTemporaryAttribute(.foregroundColor, forCharacterRange: full)
        if let range = range(for: id) {
            view.setSelectedRange(NSRange(location: 0, length: 0))
            layout.addTemporaryAttributes([
                .backgroundColor: NSColor.selectedTextBackgroundColor,
                .foregroundColor: NSColor.selectedTextColor
            ], forCharacterRange: range)
        }
        view.needsDisplay = true
    }

    func position(at point: NSPoint, in window: NSWindow) -> Position? {
        let views = ids.compactMap { id -> (String, IrisMessageTextView, NSRect)? in
            guard let view = entries[id]?.view, view.window === window, !view.isHidden,
                  view.bounds.height > 0 else { return nil }
            return (id, view, view.convert(view.bounds, to: nil))
        }
        // Window coordinates increase upward; choose the closest text row, then
        // let TextKit resolve wrapping, bidirectional text and insertion points.
        guard let (id, view, frame) = views.min(by: {
            distance(point.y, $0.2) < distance(point.y, $1.2)
        }) else { return nil }
        let offset: Int
        if point.y > frame.maxY { offset = 0 }
        else if point.y < frame.minY { offset = (view.string as NSString).length }
        else { offset = view.characterIndexForInsertion(at: view.convert(point, from: nil)) }
        return Position(id: id, offset: offset)
    }

    private func distance(_ y: CGFloat, _ frame: NSRect) -> CGFloat {
        max(frame.minY - y, y - frame.maxY, 0)
    }
}

final class IrisMessageTextView: NSTextView {
    weak var messageSelection: IrisDesktopMessageSelection?
    var messageID: String?

    override func mouseDown(with event: NSEvent) {
        guard let selection = messageSelection, let id = messageID, let window,
              event.clickCount == 1 else {
            messageSelection?.clear()
            super.mouseDown(with: event)
            return
        }
        let extending = event.modifierFlags.contains(.shift) && selection.anchor != nil
        let start = extending ? selection.anchor! : .init(
            id: id, offset: characterIndexForInsertion(at: convert(event.locationInWindow, from: nil)))
        if !extending { selection.clear() }

        // Leave clicks (including links) and native double/triple clicks to AppKit.
        // Peek first: once dragging starts, the conversation owns the gesture.
        if !extending, let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp],
            until: .distantFuture, inMode: .eventTracking, dequeue: false), next.type == .leftMouseUp {
            super.mouseDown(with: event)
            return
        }
        window.makeFirstResponder(self)
        setSelectedRange(NSRange(location: start.id == id ? start.offset : 0, length: 0))
        var point = event.locationInWindow
        while true {
            let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp],
                until: Date(timeIntervalSinceNow: 1.0 / 30), inMode: .eventTracking, dequeue: true)
            if let next { point = next.locationInWindow }
            if let end = selection.position(at: point, in: window) { selection.select(from: start, to: end) }
            if next?.type == .leftMouseUp || self.window !== window { break }
            if let scroll = enclosingScrollView {
                let clip = scroll.contentView
                let cursor = clip.convert(point, from: nil)
                let bounds = clip.bounds
                let margin: CGFloat = 24
                let delta = cursor.y < bounds.minY + margin ? -12.0 : cursor.y > bounds.maxY - margin ? 12.0 : 0
                if delta != 0 {
                    var origin = bounds.origin
                    origin.y = max(0, min(origin.y + delta, max(0, (scroll.documentView?.bounds.height ?? 0) - bounds.height)))
                    clip.scroll(to: origin)
                    scroll.reflectScrolledClipView(clip)
                }
            }
        }
    }

    override func copy(_ sender: Any?) {
        guard let text = messageSelection?.selectedText, !text.isEmpty else { super.copy(sender); return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(copy(_:)), messageSelection?.selectedText.isEmpty == false { return true }
        return super.validateUserInterfaceItem(item)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard messageSelection?.selectedText.isEmpty == false else { return super.menu(for: event) }
        let menu = NSMenu()
        let copy = menu.addItem(withTitle: "Copy", action: #selector(copy(_:)), keyEquivalent: "")
        copy.target = self
        return menu
    }

    override func cancelOperation(_ sender: Any?) {
        messageSelection?.clear()
        setSelectedRange(NSRange(location: selectedRange().location, length: 0))
    }

    override func keyDown(with event: NSEvent) {
        if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers == "c",
           messageSelection?.selectedText.isEmpty == false { copy(nil); return }
        messageSelection?.clear()
        super.keyDown(with: event)
    }
}

private struct IrisMessageSelectionKey: EnvironmentKey {
    static let defaultValue: IrisDesktopMessageSelection? = nil
}

extension EnvironmentValues {
    var irisMessageSelection: IrisDesktopMessageSelection? {
        get { self[IrisMessageSelectionKey.self] }
        set { self[IrisMessageSelectionKey.self] = newValue }
    }
}

/// Own event monitoring at the conversation's lifetime, never globally or on a
/// timer. Clicking the composer or switching chats clears the range.
struct IrisDesktopSelectionScope: NSViewRepresentable {
    let selection: IrisDesktopMessageSelection
    let chatID: String
    let ids: [String]

    func makeNSView(context: Context) -> ScopeView { ScopeView(selection: selection) }
    func updateNSView(_ view: ScopeView, context: Context) { selection.update(chatID: chatID, ids: ids) }

    final class ScopeView: NSView {
        let selection: IrisDesktopMessageSelection
        private var monitor: Any?
        init(selection: IrisDesktopMessageSelection) { self.selection = selection; super.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor); self.monitor = nil }
            guard window != nil else { selection.clear(); return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
                guard let self, let window = self.window, event.window === window else { return event }
                let hit = window.contentView?.hitTest(event.locationInWindow)
                if (hit as? IrisMessageTextView)?.messageSelection !== self.selection { self.selection.clear() }
                return event
            }
        }
        deinit { if let monitor { NSEvent.removeMonitor(monitor) } }
    }
}
#endif
