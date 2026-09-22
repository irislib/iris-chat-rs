import Foundation
import UserNotifications

/// iOS Notification Service Extension. Receives the encrypted Nostr
/// event the notification server forwarded, decrypts it against the
/// persisted double-ratchet state in the App Group container, and
/// rewrites the visible notification with the chat name as the title
/// and the plaintext message as the body. Group notifications use the
/// group name as title and prefix the body with the sender name. If
/// decryption fails for any reason — no logged-in
/// account, missing storage, ratchet already advanced by the foreground
/// app — use a quiet background-update placeholder. iOS rejects blank
/// content without the notification-filtering entitlement.
final class NotificationService: UNNotificationServiceExtension {
    private static let appGroupIdentifier = "group.fi.siriusbusiness.irischat"
    private static let keychainService = "fi.siriusbusiness.irischat"
    private static let keychainAccount = "stored-account-bundle"
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        MobilePushDeliveryProbe.recordIfArmed()
        self.contentHandler = contentHandler
        let bestAttempt = MobilePushPresentation.fallbackContent(from: request.content)
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

        let resolved = MobilePushPresentation.resolvedContent(
            from: request.content, resolution: resolution
        )
        self.bestAttempt = resolved
        contentHandler(resolved)
    }

    override func serviceExtensionTimeWillExpire() {
        // Apple gives the NSE ~30s. Hand off whatever we managed to
        // mutate; the prepared fallback stays quiet if decryption times out.
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
