import Foundation

extension ChatMessageSnapshot: IrisInteractionMessage {}

extension AppManager {
    func recordInteractionState(historyLoaded: Bool = false) {
        guard let timing = interactionTiming, let chat = state.currentChat else { return }
        timing.stateAvailable(chatID: chat.chatId, messages: chat.messages, historyLoaded: historyLoaded)
    }

    func recordInteractionAction(_ action: AppAction) {
        guard let timing = interactionTiming else { return }
        switch action {
        case .openChat(let chatID):
            timing.beginOpen(chatID: chatID.trimmingCharacters(in: .whitespacesAndNewlines),
                             targetID: pendingScrollMessageId)
        case .sendMessage(let chatID, let text), .sendDisappearingMessage(let chatID, let text, _):
            timing.beginSend(chatID: chatID, body: text.trimmingCharacters(in: .whitespacesAndNewlines),
                             messages: state.currentChat?.chatId == chatID ? state.currentChat!.messages : [])
        default: break
        }
    }
}
