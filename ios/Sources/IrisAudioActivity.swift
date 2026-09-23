import Foundation

/// Shared call ownership for voice-note capture and attachment playback.
/// Serializes a voice-note session's setup with the call ownership handoff.
enum IrisAudioActivity {
    static let callDidChange = Notification.Name("IrisCallAudioActivityChanged")
    private static let lock = NSLock()
    private static var active = false
    private static var recording = false

    static var isRecordingActive: Bool {
        lock.lock()
        defer { lock.unlock() }
        return recording
    }

    static func setRecordingActive(_ value: Bool) {
        lock.lock()
        recording = value
        lock.unlock()
    }

    static var isCallActive: Bool {
        lock.lock()
        defer { lock.unlock() }
        return active
    }

    static func setCallActive(_ value: Bool) {
        lock.lock()
        let changed = active != value
        active = value
        lock.unlock()
        if changed { NotificationCenter.default.post(name: callDidChange, object: nil) }
    }

    static func withRecordingSessionIfNoCall<T>(_ body: () throws -> T) rethrows -> T? {
        lock.lock()
        defer { lock.unlock() }
        guard !active else { return nil }
        return try body()
    }

    static func withPlaybackSessionIfAvailable<T>(_ body: () throws -> T) rethrows -> T? {
        lock.lock()
        defer { lock.unlock() }
        guard !active && !recording else { return nil }
        return try body()
    }
}
