import SwiftUI

#if os(macOS)
import AppKit

struct DesktopKeyboardChatList<Item, Row: View>: View {
    let items: [Item]
    let id: KeyPath<Item, String>
    let selectedChatID: String?
    let proxy: ScrollViewProxy
    let onOpen: (String) -> Void
    @ViewBuilder let row: (Item) -> Row
    @Environment(\.desktopChatListFocusRequest) private var focusRequest
    @State private var focusedChatID: String?
    @State private var listFocused = false
    @State private var viewport = DesktopChatListViewport()

    private var ids: [String] { items.map { $0[keyPath: id] } }

    var body: some View {
        LazyVStack(spacing: 2) {
            ForEach(items, id: id) { item in
                let chatID = item[keyPath: id]
                row(item)
                    .environment(\.irisKeyboardButtonIsTabStop, false)
                    .overlay {
                        if listFocused && focusedChatID == chatID {
                            RoundedRectangle(cornerRadius: 10)
                                .strokeBorder(Color.accentColor, lineWidth: 2)
                                .allowsHitTesting(false)
                        }
                    }
                    .id(chatID)
            }
        }
        .background(DesktopChatListViewportReader(view: viewport, onFocus: { focused in
            listFocused = focused
            if focused && focusedChatID == nil { focusRow(selectedChatID ?? ids.first) }
        }, onTab: moveFocus, onOpen: {
            if let focusedChatID { onOpen(focusedChatID) }
        }))
        .accessibilityIdentifier("desktopChatKeyboardList")
        .onAppear(perform: applyFocusRequest)
        .onChange(of: focusRequest.wrappedValue) { _, _ in applyFocusRequest() }
        .onChange(of: ids) { _, values in
            if let focusedChatID, !values.contains(focusedChatID) {
                focusRow(values.contains(selectedChatID ?? "") ? selectedChatID : values.first)
            }
        }
    }

    private func focusRow(_ chatID: String?) {
        focusedChatID = chatID
        if let chatID { proxy.scrollTo(chatID, anchor: .center) }
    }

    private func moveFocus(_ offset: Int) -> Bool {
        guard let current = focusedChatID.flatMap({ ids.firstIndex(of: $0) }) else {
            focusRow(ids.first)
            return !ids.isEmpty
        }
        let next = current + offset
        guard ids.indices.contains(next) else { return false }
        focusRow(ids[next])
        return true
    }

    private func applyFocusRequest() {
        guard let request = focusRequest.wrappedValue else { return }
        DispatchQueue.main.async {
            guard focusRequest.wrappedValue == request else { return }
            focusRow(selectedChatID.flatMap { ids.contains($0) ? $0 : nil } ?? ids.first)
            viewport.window?.makeFirstResponder(viewport)
            focusRequest.wrappedValue = nil
        }
    }
}

private struct DesktopChatListViewportReader: NSViewRepresentable {
    let view: DesktopChatListViewport
    let onFocus: (Bool) -> Void
    let onTab: (Int) -> Bool
    let onOpen: () -> Void
    func makeNSView(context: Context) -> DesktopChatListViewport { view }
    func updateNSView(_ view: DesktopChatListViewport, context: Context) {
        view.onFocus = onFocus
        view.onTab = onTab
        view.onOpen = onOpen
    }
    static func dismantleNSView(_ view: DesktopChatListViewport, coordinator: ()) {
        view.onFocus = nil
        view.onTab = nil
        view.onOpen = nil
    }
}

/// Focus belongs to the list, so scrolling a lazy row offscreen cannot lose it.
final class DesktopChatListViewport: NSView {
    var onFocus: ((Bool) -> Void)?
    var onTab: ((Int) -> Bool)?
    var onOpen: (() -> Void)?
    override var acceptsFirstResponder: Bool { true }
    override var canBecomeKeyView: Bool { true }
    override func becomeFirstResponder() -> Bool { notifyFocus(); return true }
    override func resignFirstResponder() -> Bool { notifyFocus(); return true }

    private func notifyFocus() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            onFocus?(window?.firstResponder === self)
        }
    }

    override func keyDown(with event: NSEvent) {
        guard event.modifierFlags.intersection([.command, .option, .control]).isEmpty else {
            super.keyDown(with: event)
            return
        }
        if event.keyCode == 48 {
            let backward = event.modifierFlags.contains(.shift)
            if onTab?(backward ? -1 : 1) != true {
                if backward { window?.selectKeyView(preceding: self) }
                else { window?.selectKeyView(following: self) }
            }
        } else if event.modifierFlags.contains(.shift) {
            super.keyDown(with: event)
        } else {
            switch event.keyCode {
            case 125: scroll(.downArrow)
            case 126: scroll(.upArrow)
            case 115: scroll(.home)
            case 119: scroll(.end)
            case 36, 49: onOpen?()
            default: super.keyDown(with: event)
            }
        }
    }

    private func scroll(_ key: KeyEquivalent) {
        guard let scrollView = enclosingScrollView, let document = scrollView.documentView else { return }
        let clip = scrollView.contentView
        let maximum = max(0, document.bounds.height - clip.bounds.height)
        var origin = clip.bounds.origin
        switch key {
        case .home: origin.y = document.isFlipped ? 0 : maximum
        case .end: origin.y = document.isFlipped ? maximum : 0
        default:
            let direction: CGFloat = key == .downArrow ? 1 : -1
            origin.y += direction * (document.isFlipped ? 40 : -40)
        }
        origin.y = min(maximum, max(0, origin.y))
        clip.scroll(to: origin)
        scrollView.reflectScrolledClipView(clip)
    }
}
#endif
