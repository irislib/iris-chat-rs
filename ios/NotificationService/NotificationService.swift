import Foundation
import UserNotifications

/// iOS Notification Service Extension. Receives the encrypted Nostr
/// event the notification server forwarded, decrypts it against the
/// persisted double-ratchet state in the App Group container, and
/// rewrites the visible notification with the sender's display name
/// (or "<sender> in <group>") as the title and the plaintext message
/// as the body. If decryption fails for any reason — no logged-in
/// account, missing storage, ratchet already advanced by the foreground
/// app — the original (generic) notification is delivered as-is.
final class NotificationService: UNNotificationServiceExtension {
    private static let appGroupIdentifier = "group.to.iris.chat"
    private static let keychainService = "to.iris.chat"
    private static let keychainAccount = "stored-account-bundle"

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        let bestAttempt = (request.content.mutableCopy() as? UNMutableNotificationContent)
            ?? UNMutableNotificationContent()
        self.bestAttempt = bestAttempt

        guard let payloadJson = serializedPayload(from: request.content) else {
            contentHandler(bestAttempt)
            return
        }

        let resolution: MobilePushNotificationResolution
        if let bundle = loadAccountBundle(), let dataDir = sharedDataDir() {
            resolution = decryptMobilePushNotificationPayload(
                dataDir: dataDir.path,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec,
                rawPayloadJson: payloadJson
            )
        } else {
            resolution = resolveMobilePushNotificationPayload(rawPayloadJson: payloadJson)
        }

        if !resolution.shouldShow {
            // Suppress: deliver an empty / silent notification.
            contentHandler(UNMutableNotificationContent())
            return
        }
        if !resolution.title.isEmpty {
            bestAttempt.title = resolution.title
        }
        if !resolution.body.isEmpty {
            bestAttempt.body = resolution.body
        }
        contentHandler(bestAttempt)
    }

    override func serviceExtensionTimeWillExpire() {
        // Apple gives the NSE ~30s. Hand off whatever we managed to
        // mutate so the user at least gets the original notification.
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }

    private func serializedPayload(from content: UNNotificationContent) -> String? {
        let userInfo = content.userInfo
        var dict: [String: Any] = [:]
        for (key, value) in userInfo {
            guard let key = key as? String else {
                continue
            }
            dict[key] = value
        }
        if !dict.keys.contains("title") {
            dict["title"] = content.title
        }
        if !dict.keys.contains("body") {
            dict["body"] = content.body
        }
        guard JSONSerialization.isValidJSONObject(dict),
              let data = try? JSONSerialization.data(withJSONObject: dict),
              let json = String(data: data, encoding: .utf8) else {
            return nil
        }
        return json
    }

    private func sharedDataDir() -> URL? {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: Self.appGroupIdentifier
        ) else {
            return nil
        }
        return container.appendingPathComponent("iris-chat", isDirectory: true)
    }

    private struct AccountBundle: Decodable {
        let ownerNsec: String?
        let ownerPubkeyHex: String
        let deviceNsec: String
    }

    private func loadAccountBundle() -> AccountBundle? {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.keychainService,
            kSecAttrAccount: Self.keychainAccount,
            kSecReturnData: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }
        return try? JSONDecoder().decode(AccountBundle.self, from: data)
    }
}
