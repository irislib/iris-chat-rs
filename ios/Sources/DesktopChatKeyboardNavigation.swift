import SwiftUI

#if os(macOS)

struct DesktopKeyboardChatList: View {
    @Environment(\.irisPalette) private var palette
    let manager: AppManager
    let chats: [ChatThreadSnapshot]
    let selectedChatID: String?
    let preferences: PreferencesSnapshot
    let relativeNow: Date
    let proxy: ScrollViewProxy
    let onOpen: (String) -> Void
    @State private var selection = DesktopChatKeyboardSelection()
    @FocusState private var focused: Bool

    private var ids: [String] { chats.map(\.chatId) }

    var body: some View {
        LazyVStack(spacing: 2) {
            ForEach(chats, id: \.chatId) { chat in
                DesktopSidebarChatRow(
                    manager: manager,
                    chat: chat,
                    timeLabel: irisRelativeTime(chat.lastMessageAtSecs, relativeTo: relativeNow),
                    selected: selectedChatID == chat.chatId,
                    preferences: preferences
                )
                .equatable()
                .environment(\.irisKeyboardButtonIsTabStop, false)
                .overlay {
                    if focused && selection.chatID == chat.chatId {
                        RoundedRectangle(cornerRadius: 10)
                            .strokeBorder(palette.accent, lineWidth: 2)
                            .allowsHitTesting(false)
                    }
                }
                .id(chat.chatId)
                .accessibilityIdentifier("chatRow-\(String(chat.chatId.prefix(12)))")
            }
        }
        .focusable(!chats.isEmpty, interactions: .edit)
        .focusEffectDisabled()
        .focused($focused)
        .accessibilityIdentifier("desktopChatKeyboardList")
        .onChange(of: focused) { _, value in
            if value { reconcileSelection() }
        }
        .onChange(of: ids) { _, _ in reconcileSelection() }
        .onKeyPress(keys: [.upArrow, .downArrow, .home, .end]) { press in
            guard press.modifiers.intersection([.command, .option, .control, .shift]).isEmpty
            else { return .ignored }
            let offset = switch press.key {
            case .home: -ids.count
            case .end: ids.count
            case .upArrow: -1
            default: 1
            }
            selection.move(offset, in: ids, selected: selectedChatID)
            revealSelection()
            return .handled
        }
        .onKeyPress(keys: [.return, .space], phases: .down) { press in
            guard press.modifiers.intersection([.command, .option, .control, .shift]).isEmpty
            else { return .ignored }
            selection.reconcile(ids, selected: selectedChatID)
            guard let chatID = selection.chatID else { return .ignored }
            onOpen(chatID)
            return .handled
        }
    }

    private func reconcileSelection() {
        selection.reconcile(ids, selected: selectedChatID)
        if focused { revealSelection() }
    }

    private func revealSelection() {
        if let chatID = selection.chatID { proxy.scrollTo(chatID, anchor: .center) }
    }
}
#endif
