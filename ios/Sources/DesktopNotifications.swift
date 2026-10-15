import Foundation
import UserNotifications

protocol DesktopNotificationPosting {
    func post(accountID: String, chatID: String, title: String, body: String)
    func clearRead(accountID: String, chatIDs: Set<String>)
    func clear()
    func post(accountID: String, chatID: String, messageID: String?, dataDir: String, title: String, body: String)
    func clearRead(accountID: String, dataDir: String, fullyReadChatIDs: Set<String>)
}

extension DesktopNotificationPosting {
    func post(accountID: String, chatID: String, messageID: String?, dataDir: String, title: String, body: String) {
        post(accountID: accountID, chatID: chatID, title: title, body: body)
    }
    func clearRead(accountID: String, dataDir: String, fullyReadChatIDs: Set<String>) {
        if !fullyReadChatIDs.isEmpty { clearRead(accountID: accountID, chatIDs: fullyReadChatIDs) }
    }
}

final class NoopDesktopNotificationPoster: DesktopNotificationPosting {
    func post(accountID: String, chatID: String, title: String, body: String) {
        _ = accountID
        _ = chatID
        _ = title
        _ = body
    }
    func clear() {}
    func clearRead(accountID: String, chatIDs: Set<String>) {}
}

final class SystemDesktopNotificationPoster: DesktopNotificationPosting {
    private let center = UNUserNotificationCenter.current()
    private let environment: [String: String]
    private let lock = NSLock()
    private var generation = 0

    init(environment: [String: String] = ProcessInfo.processInfo.environment) {
        self.environment = environment
    }

    func post(accountID: String, chatID: String, title: String, body: String) {
        post(accountID: accountID, chatID: chatID, messageID: nil, dataDir: "", title: title, body: body)
    }

    func post(accountID: String, chatID: String, messageID: String?, dataDir: String, title: String, body: String) {
        guard !AppPaths.notificationsDisabledForAutomation(environment: environment) else {
            return
        }
        lock.lock()
        let requestGeneration = generation
        lock.unlock()
        let isCurrent = { [weak self] in
            guard let self else { return false }
            self.lock.lock()
            defer { self.lock.unlock() }
            return self.generation == requestGeneration
        }
        center.getNotificationSettings { [center] settings in
            guard isCurrent() else { return }
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                Self.enqueue(accountID: accountID, chatID: chatID, messageID: messageID, dataDir: dataDir, title: title, body: body, center: center, isCurrent: isCurrent)
            case .notDetermined:
                center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                    guard granted, isCurrent() else {
                        return
                    }
                    Self.enqueue(accountID: accountID, chatID: chatID, messageID: messageID, dataDir: dataDir, title: title, body: body, center: center, isCurrent: isCurrent)
                }
            case .denied:
                break
            @unknown default:
                break
            }
        }
    }

    func clearRead(accountID: String, chatIDs: Set<String>) {
        clearRead(accountID: accountID, dataDir: "", fullyReadChatIDs: chatIDs)
    }

    func clearRead(accountID: String, dataDir: String, fullyReadChatIDs: Set<String>) {
        guard !AppPaths.notificationsDisabledForAutomation(environment: environment) else { return }
        let requestedAt = Date()
        center.getDeliveredNotifications { [center] notifications in
            let records = notifications.filter { $0.date <= requestedAt }.map { DesktopReadNotification($0.request) }
            let ids = DesktopReadNotification.readIdentifiers(records, accountID: accountID, dataDir: dataDir, fullyReadChatIDs: fullyReadChatIDs)
            if !ids.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ids) }
        }
        center.getPendingNotificationRequests { [center] requests in
            // Exact message IDs remain safe if a notification was enqueued after
            // this snapshot. Legacy pending entries have no such proof.
            let ids = DesktopReadNotification.readIdentifiers(requests.map(DesktopReadNotification.init), accountID: accountID, dataDir: dataDir, fullyReadChatIDs: [])
            if !ids.isEmpty { center.removePendingNotificationRequests(withIdentifiers: ids) }
        }
    }

    func clear() {
        lock.lock()
        generation += 1
        lock.unlock()
        guard !AppPaths.notificationsDisabledForAutomation(environment: environment) else { return }
        center.removeAllPendingNotificationRequests()
        center.removeAllDeliveredNotifications()
    }

    static func content(accountID: String, chatID: String, messageID: String? = nil, title: String, body: String) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.userInfo = ["chatId": chatID, "iris_account_id": accountID]
        if let messageID { content.userInfo["iris_message_id"] = messageID }
        return content
    }

    private static func enqueue(accountID: String, chatID: String, messageID: String?, dataDir: String, title: String, body: String,
                                center: UNUserNotificationCenter, isCurrent: @escaping () -> Bool) {
        let wasRead = {
            guard let messageID, !dataDir.isEmpty else { return false }
            return !readDesktopNotificationIndexes(dataDir: dataDir, ownerPubkeyHex: accountID,
                messages: [DesktopNotificationMessage(chatId: chatID, messageId: messageID)]).isEmpty
        }
        guard isCurrent(), !wasRead() else { return }
        let request = UNNotificationRequest(
            identifier: "iris-chat-\(UUID().uuidString)",
            content: content(accountID: accountID, chatID: chatID, messageID: messageID, title: title, body: body),
            trigger: nil
        )
        center.add(request) { _ in
            if !isCurrent() || wasRead() {
                center.removePendingNotificationRequests(withIdentifiers: [request.identifier])
                center.removeDeliveredNotifications(withIdentifiers: [request.identifier])
            }
        }
    }
}

