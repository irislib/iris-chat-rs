import Foundation
import UserNotifications

@MainActor
final class MacUserNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    private let activate: () -> Void
    private var openChat: ((String) -> Void)?
    private var pendingChatID: String?
    private(set) var hasPendingTap = false

    init(activate: @escaping () -> Void) {
        self.activate = activate
    }

    func configure(openChat: @escaping (String) -> Void) {
        self.openChat = openChat
        guard hasPendingTap else { return }
        hasPendingTap = false
        if let pendingChatID { openChat(pendingChatID) }
        pendingChatID = nil
        activate()
    }

    func handleResponse(actionIdentifier: String, userInfo: [AnyHashable: Any]) {
        // A dismissal is not a request to open the app. Call alerts only
        // navigate to their chat; answering still requires the call controls.
        guard actionIdentifier == UNNotificationDefaultActionIdentifier else { return }
        let chatID = (userInfo["chatId"] as? String ?? userInfo["chat_id"] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let target = chatID.flatMap { $0.isEmpty ? nil : $0 }
        guard let openChat else {
            pendingChatID = target
            hasPendingTap = true
            return
        }
        if let target { openChat(target) }
        activate()
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .list])
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        Task { @MainActor in
            handleResponse(actionIdentifier: response.actionIdentifier,
                           userInfo: response.notification.request.content.userInfo)
            completionHandler()
        }
    }
}
