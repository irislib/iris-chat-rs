import Foundation
import UserNotifications

@MainActor
final class MacUserNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    private let activate: () -> Void
    private var openChat: ((String, String) -> Void)?
    private var pendingTarget: (chatID: String, accountID: String)?
    private(set) var hasPendingTap = false

    init(activate: @escaping () -> Void) {
        self.activate = activate
    }

    func configure(openChat: @escaping (String, String) -> Void) {
        self.openChat = openChat
        guard hasPendingTap else { return }
        hasPendingTap = false
        if let pendingTarget { openChat(pendingTarget.chatID, pendingTarget.accountID) }
        pendingTarget = nil
        activate()
    }

    func handleResponse(actionIdentifier: String, userInfo: [AnyHashable: Any]) {
        // A dismissal is not a request to open the app. Call alerts only
        // navigate to their chat; answering still requires the call controls.
        guard actionIdentifier == UNNotificationDefaultActionIdentifier else { return }
        let chatID = (userInfo["chatId"] as? String ?? userInfo["chat_id"] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let accountID = (userInfo["iris_account_id"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        // Old alerts without account identity may activate the window, but
        // cannot safely choose a conversation after the user switches accounts.
        let target: (String, String)? = if let chatID, !chatID.isEmpty, let accountID, !accountID.isEmpty {
            (chatID, accountID)
        } else { nil }
        guard let openChat else {
            pendingTarget = target
            hasPendingTap = true
            return
        }
        if let target { openChat(target.0, target.1) }
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
