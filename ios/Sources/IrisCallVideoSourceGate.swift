import Foundation

/// Replacing the video source revokes frames already queued by the old source.
/// This is independent of audio mute and does not restart the call transport.
final class IrisCallVideoSourceGate {
    private let lock = NSLock()
    private var screen = false
    private var generation: UInt64 = 0
    private var after: UInt64 = 0

    func select(screen: Bool) {
        lock.lock()
        defer { lock.unlock() }
        self.screen = screen
        generation &+= 1
        after = UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000)
    }

    func permission(screen: Bool, capturedAt: UInt64) -> () -> Bool {
        lock.lock()
        let generation = self.generation
        let recent = capturedAt >= after
        lock.unlock()
        return { [weak self] in
            guard let self else { return false }
            self.lock.lock()
            defer { self.lock.unlock() }
            return recent && self.screen == screen && self.generation == generation
        }
    }
}
