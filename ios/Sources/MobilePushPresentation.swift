import Foundation
import UserNotifications

enum MobilePushPresentation {
    static func fallbackContent(from original: UNNotificationContent) -> UNMutableNotificationContent {
        let content = original.mutableCopy() as? UNMutableNotificationContent
            ?? UNMutableNotificationContent()
        guard isEncryptedIrisPush(original) else { return content }

        // Without Apple's filtering entitlement, empty content is rejected and
        // iOS restores the original, audible "New message" notification.
        content.title = "Iris Chat"
        content.subtitle = ""
        content.body = "Background update"
        content.sound = nil
        content.badge = nil
        content.interruptionLevel = .passive
        content.relevanceScore = 0
        content.threadIdentifier = "iris-background-updates"
        return content
    }

    static func resolvedContent(
        from original: UNNotificationContent,
        resolution: MobilePushNotificationResolution
    ) -> UNMutableNotificationContent {
        let hasPreview = !resolution.title.isEmpty || !resolution.body.isEmpty
        guard resolution.shouldShow || hasPreview,
              !(isEncryptedIrisPush(original) && isGenericFallback(
                title: resolution.title, body: resolution.body
              )) else {
            return fallbackContent(from: original)
        }
        let content = original.mutableCopy() as? UNMutableNotificationContent
            ?? UNMutableNotificationContent()
        if !resolution.title.isEmpty { content.title = resolution.title }
        if !resolution.body.isEmpty { content.body = resolution.body }
        content.sound = resolution.shouldShow ? .default : nil
        content.interruptionLevel = resolution.shouldShow ? .active : .passive
        if !resolution.shouldShow { content.badge = nil }
        return content
    }

    static func isGenericFallback(title: String, body: String) -> Bool {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let body = body.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let genericBody = body.isEmpty || body == "new activity" || body == "new message"
        let genericTitle = title.isEmpty || title == "iris chat" || title == "new activity" ||
            title == "new message" || title == "someone" || title.hasPrefix("dm by ")
        return genericTitle && genericBody
    }

    private static func isEncryptedIrisPush(_ content: UNNotificationContent) -> Bool {
        let keys = ["event", "outer_event", "outer_event_json", "nostr_event", "nostr_event_json"]
        return keys.contains { eventKind(content.userInfo[$0]) == 1060 } ||
            isGenericFallback(title: content.title, body: content.body)
    }

    private static func eventKind(_ value: Any?) -> Int? {
        if let object = value as? [AnyHashable: Any] {
            if let number = object["kind"] as? NSNumber { return number.intValue }
            if let string = object["kind"] as? String {
                return Int(string.trimmingCharacters(in: .whitespacesAndNewlines))
            }
        }
        if let string = value as? String,
           let data = string.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            return eventKind(object)
        }
        return nil
    }
}
