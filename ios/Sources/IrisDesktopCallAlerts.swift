#if os(macOS)
import AppKit
import UserNotifications

/// The alert belongs to a call, including while Notification Center posts it.
@MainActor
final class IrisDesktopCallAlerts {
    private var callID: String?
    private var notificationID: String?
    private var timer: Timer?
    private var attention: Int?
    private let sound = NSSound(named: NSSound.Name("Glass"))

    func update(_ call: CallSnapshot?) {
        guard let call, call.phase == "incoming" else { stop(); return }
        guard callID != call.callId else { return }
        stop()
        callID = call.callId
        attention = NSApp.requestUserAttention(.criticalRequest)
        sound?.play()
        timer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
            Task { @MainActor in if self?.callID != nil { self?.sound?.play() } }
        }
        let id = "iris-call-\(UUID().uuidString)"
        notificationID = id
        let content = UNMutableNotificationContent()
        content.title = call.peerName
        content.body = call.videoCapable ? "Incoming video call" : "Incoming voice call"
        content.userInfo = ["callId": call.callId]
        // Ringtone is owned here, so notification delivery cannot leave a sound
        // playing after an answer, decline, remote cancellation or timeout.
        Task { [weak self] in
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            guard self?.notificationID == id else { return }
            if settings.authorizationStatus == .notDetermined {
                _ = try? await center.requestAuthorization(options: [.alert, .sound])
            }
            guard self?.notificationID == id else { return }
            try? await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
            if self?.notificationID != id {
                center.removePendingNotificationRequests(withIdentifiers: [id])
                center.removeDeliveredNotifications(withIdentifiers: [id])
            }
        }
    }

    private func stop() {
        callID = nil
        timer?.invalidate(); timer = nil
        sound?.stop()
        if let attention { NSApp.cancelUserAttentionRequest(attention) }
        attention = nil
        if let id = notificationID {
            UNUserNotificationCenter.current().removePendingNotificationRequests(withIdentifiers: [id])
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
        }
        notificationID = nil
    }

    deinit {
        timer?.invalidate()
        sound?.stop()
        if let attention { Task { @MainActor in NSApp.cancelUserAttentionRequest(attention) } }
        if let id = notificationID {
            UNUserNotificationCenter.current().removePendingNotificationRequests(withIdentifiers: [id])
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
        }
    }
}
#endif
