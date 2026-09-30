import UserNotifications

/// Without Apple's filtering entitlement an empty alert restores the original
/// server placeholder. Keep a truthful, quiet fallback until filtering is enabled.
enum MobilePushNotificationPresentation {
    // Enable only in builds signed with the notification-filtering entitlement.
    static let filteringEnabled = false

    static func prepareFallback(_ content: UNMutableNotificationContent, canFilter: Bool = filteringEnabled) {
        content.title = canFilter ? "" : "Iris Chat"
        content.subtitle = ""
        content.body = canFilter ? "" : "Chat updated"
        content.sound = nil
        content.badge = nil
    }

    static func apply(
        _ resolution: MobilePushNotificationResolution,
        to content: UNMutableNotificationContent,
        canFilter: Bool = filteringEnabled
    ) {
        if !resolution.shouldShow && canFilter {
            prepareFallback(content, canFilter: true)
            return
        }
        if !resolution.title.isEmpty { content.title = resolution.title }
        if !resolution.body.isEmpty { content.body = resolution.body }
        content.sound = resolution.shouldShow ? .default : nil
        if !resolution.shouldShow { content.badge = nil }
    }
}
