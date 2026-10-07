import Foundation
import UserNotifications

/// Decrypts message previews using the shared core's notification policy.
/// Controls, muted/read/outgoing messages, and unresolved encrypted pushes
/// return empty content under Apple's notification-filtering entitlement.
final class NotificationService: UNNotificationServiceExtension {
    private static let appGroupIdentifier = "group.fi.siriusbusiness.irischat"
    private static let keychainService = "fi.siriusbusiness.irischat"
    private static let keychainAccount = "stored-account-bundle"
    private static let encryptedEventPayloadKeys = [
        "event",
        "outer_event",
        "outer_event_json",
        "nostr_event",
        "nostr_event_json",
    ]

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private let completionLock = NSLock()

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        MobilePushDeliveryProbe.recordIfArmed(payloadID: request.content.userInfo["iris_push_e2e_id"] as? String)
        completionLock.lock()
        self.contentHandler = contentHandler
        completionLock.unlock()
        let bestAttempt = (request.content.mutableCopy() as? UNMutableNotificationContent)
            ?? UNMutableNotificationContent()
        let shouldClearFallback = isLikelyEncryptedIrisPush(request.content)

        guard let payloadJson = serializedPayload(from: request.content) else {
            finish(UNNotificationContent())
            return
        }

        let resolution: MobilePushNotificationResolution
        if let bundle = loadAccountBundle(), let dataDir = sharedDataDir() {
            bestAttempt.userInfo["iris_account_id"] = bundle.ownerPubkeyHex
            resolution = decryptMobilePushNotificationPayload(
                dataDir: dataDir.path,
                ownerPubkeyHex: bundle.ownerPubkeyHex,
                deviceNsec: bundle.deviceNsec,
                rawPayloadJson: payloadJson
            )
        } else {
            resolution = resolveMobilePushNotificationPayload(rawPayloadJson: payloadJson)
        }

        if shouldClearFallback && isGenericFallbackResolution(resolution) {
            finish(UNNotificationContent())
            return
        }
        finish(MobilePushNotificationPresentation.content(for: resolution, original: bestAttempt))
    }

    override func serviceExtensionTimeWillExpire() {
        // Never restore the server placeholder if decryption runs out of time.
        finish(UNNotificationContent())
    }

    private func finish(_ content: UNNotificationContent) {
        completionLock.lock()
        let handler = contentHandler
        contentHandler = nil
        completionLock.unlock()
        // Expiration and decryption can race; deliver exactly once.
        handler?(content)
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

    private func isLikelyEncryptedIrisPush(_ content: UNNotificationContent) -> Bool {
        for key in Self.encryptedEventPayloadKeys {
            if eventKind(content.userInfo[key]) == 1060 {
                return true
            }
        }
        return isGenericIrisFallback(content)
    }

    private func eventKind(_ value: Any?) -> Int? {
        if let dict = value as? [String: Any] {
            return normalizedInt(dict["kind"])
        }
        if let dict = value as? [AnyHashable: Any] {
            return normalizedInt(dict["kind"])
        }
        if let string = value as? String,
           let data = string.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            return normalizedInt(object["kind"])
        }
        return nil
    }

    private func normalizedInt(_ value: Any?) -> Int? {
        if let intValue = value as? Int {
            return intValue
        }
        if let number = value as? NSNumber {
            return number.intValue
        }
        if let string = value as? String {
            return Int(string.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return nil
    }

    private func isGenericIrisFallback(_ content: UNNotificationContent) -> Bool {
        isGenericFallback(title: content.title, body: content.body)
    }

    private func isGenericFallbackResolution(_ resolution: MobilePushNotificationResolution) -> Bool {
        isGenericFallback(title: resolution.title, body: resolution.body)
    }

    private func isGenericFallback(title: String, body: String) -> Bool {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let body = body.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let genericBody = body.isEmpty || body == "new activity" || body == "new message"
        let genericTitle = title.isEmpty ||
            title == "iris chat" ||
            title == "new activity" ||
            title == "new message" ||
            title == "someone" ||
            title.hasPrefix("dm by ")
        return genericTitle && genericBody
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
