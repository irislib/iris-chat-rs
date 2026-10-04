import Foundation

func nearbyPeerChat(owner: String?, chats: [ChatThreadSnapshot]) -> ChatThreadSnapshot? {
    guard let owner = owner?.trimmingCharacters(in: .whitespacesAndNewlines), !owner.isEmpty else {
        return nil
    }
    return chats.first { $0.kind == .direct && $0.chatId.caseInsensitiveCompare(owner) == .orderedSame }
}
