import SwiftUI

extension CurrentChatSnapshot {
    var isRemovedFromGroup: Bool {
        kind == .group && !participants.contains(where: \.isLocalOwner)
    }
}

struct IrisRemovedGroupBar: View {
    var body: some View {
        Text("You’re no longer in this group")
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 16)
            .padding(.vertical, 18)
            .background(.regularMaterial)
            .accessibilityIdentifier("removedGroupBar")
    }
}

extension AppManager {
    func shouldBlockOutgoingAction(_ action: AppAction) -> Bool {
        switch action {
        case .sendMessage(chatId: let chatId, text: _),
             .sendDisappearingMessage(chatId: let chatId, text: _, expiresAtSecs: _),
             .sendAttachment(chatId: let chatId, filePath: _, filename: _, caption: _),
             .sendAttachments(chatId: let chatId, attachments: _, caption: _),
             .sendDirectFiles(chatId: let chatId, attachments: _, caption: _),
             .acceptDirectFiles(chatId: let chatId, transferId: _),
             .sendTyping(chatId: let chatId),
             .toggleReaction(chatId: let chatId, messageId: _, emoji: _):
            return shouldBlockOutgoingChat(chatId: chatId)
        default:
            return false
        }
    }

    func shouldBlockOutgoingChat(chatId: String) -> Bool {
        let trimmed = chatId.trimmingCharacters(in: .whitespacesAndNewlines)
        if state.currentChat?.chatId == trimmed, state.currentChat?.isRemovedFromGroup == true {
            return true
        }
        guard !trimmed.isEmpty, !trimmed.lowercased().hasPrefix("group:") else {
            return false
        }
        return isUserBlocked(trimmed)
    }
}
