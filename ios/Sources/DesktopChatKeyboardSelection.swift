#if os(macOS)
struct DesktopChatKeyboardSelection {
    private(set) var chatID: String?

    mutating func reconcile(_ ids: [String], selected: String?) {
        if let chatID, ids.contains(chatID) { return }
        chatID = selected.flatMap { ids.contains($0) ? $0 : nil } ?? ids.first
    }

    mutating func move(_ offset: Int, in ids: [String], selected: String?) {
        reconcile(ids, selected: selected)
        guard let chatID, let index = ids.firstIndex(of: chatID) else { return }
        self.chatID = ids[min(max(index + offset, 0), ids.count - 1)]
    }
}

#endif
