import Foundation

/// Permission belongs to the original capture epoch, not the time a queued
/// callback finally executes. Audio and video have independent privacy epochs.
final class IrisCallSendGate: @unchecked Sendable {
    private let lock = NSLock()
    private var callID: String?
    private var muted = true
    private var video = false
    private var audioGeneration: UInt64 = 0
    private var videoGeneration: UInt64 = 0
    private var audioAfter: UInt64 = 0
    private var videoAfter: UInt64 = 0

    func update(callID: String?, muted: Bool, video: Bool) {
        lock.lock()
        defer { lock.unlock() }
        let now = UInt64(ProcessInfo.processInfo.systemUptime * 1_000_000)
        if self.callID != callID || self.muted != muted { audioGeneration &+= 1; audioAfter = now }
        if self.callID != callID || self.video != video { videoGeneration &+= 1; videoAfter = now }
        self.callID = callID
        self.muted = muted
        self.video = video
    }

    func permission(callID: String, kind: UInt8, capturedAtUs: UInt64? = nil) -> () -> Bool {
        lock.lock()
        let generation = kind == 1 ? audioGeneration : videoGeneration
        let recent = capturedAtUs.map { $0 >= (kind == 1 ? audioAfter : videoAfter) } ?? true
        lock.unlock()
        return { [weak self] in
            guard let self else { return false }
            self.lock.lock()
            defer { self.lock.unlock() }
            return recent && generation == (kind == 1 ? self.audioGeneration : self.videoGeneration) &&
                self.callID == callID && (kind == 1 ? !self.muted : self.video)
        }
    }
}
