import UserNotifications

/// The extension is signed with Apple's notification-filtering entitlement.
/// Suppression requires a fresh, empty content object, including no payload,
/// badge, sound, or notification actions from the server placeholder.
enum MobilePushNotificationPresentation {
    static func content(
        for resolution: MobilePushNotificationResolution,
        original: UNNotificationContent
    ) -> UNNotificationContent {
        guard resolution.shouldShow else { return UNNotificationContent() }
        let content = (original.mutableCopy() as? UNMutableNotificationContent)
            ?? UNMutableNotificationContent()
        content.title = resolution.title
        content.subtitle = ""
        content.body = resolution.body
        content.sound = .default
        return content
    }
}
