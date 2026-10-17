import Foundation

/// Coalesce a capture burst into one scheduled drain. Keep the earliest frames
/// in order on overload: one draining batch plus one pending batch, each
/// bounded to 24 audio / 4 video frames. The caller checks age/permission
/// before each dispatch; buffering never extends a frame's deadline.
final class IrisCallMediaHandoff<Element>: @unchecked Sendable {
    private let lock = NSLock()
    private var pending: [Element] = []
    private var audioCount = 0
    private var videoCount = 0
#if DEBUG
    var continuityEnqueued: IrisCallAudioContinuityProbe.Stage = .controllerEnqueued
    var continuityFull: IrisCallAudioContinuityProbe.Stage = .controllerFull
#endif

    /// True only when the caller must schedule a drain; never waits for it.
    func append(_ element: Element, kind: UInt8) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        switch kind {
        case 1:
            guard audioCount < 24 else {
#if DEBUG
                IrisCallAudioContinuityProbe.shared.event(continuityFull)
#endif
                return false
            }
#if DEBUG
            IrisCallAudioContinuityProbe.shared.event(continuityEnqueued)
#endif
            audioCount += 1
        case 2:
            guard videoCount < 4 else { return false }
            videoCount += 1
        default: return false
        }
        let schedule = pending.isEmpty
        pending.append(element)
        return schedule
    }

    func take() -> [Element] {
        lock.lock()
        defer { lock.unlock() }
        let batch = pending
        pending = []
        audioCount = 0
        videoCount = 0
        return batch
    }
}

/// The native controller's final asynchronous handoff to the Rust core.
final class IrisCallMediaDispatch {
    private let queue: DispatchQueue
    private let pending = IrisCallMediaHandoff<() -> Void>()
    private let now: () -> TimeInterval
    private let lock = NSLock()
    private var generation: UInt64 = 0

    init(queue: DispatchQueue = DispatchQueue(label: "iris.call.send", qos: .userInteractive),
         now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.queue = queue
        self.now = now
#if DEBUG
        pending.continuityEnqueued = .dispatchEnqueued
        pending.continuityFull = .dispatchFull
#endif
    }

    /// Invalidate the old core's work before account/core replacement begins.
    func invalidate() {
        lock.lock()
        generation &+= 1
        lock.unlock()
    }

    private func isCurrent(_ generation: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return self.generation == generation
    }

    func send(_ action: AppAction, allowed: @escaping () -> Bool, dispatch: @escaping (AppAction) -> Void) {
        guard case .sendCallMedia(_, let kind, _, _, _) = action else { return }
        lock.lock()
        let generation = self.generation
        lock.unlock()
        let deadline = now() + 0.1
        let work = { [weak self, now] in
            // allowed also retains the controller's original 100ms deadline;
            // the second queue cannot grant captured audio a fresh age budget.
            guard let self, self.isCurrent(generation), now() < deadline, allowed() else {
#if DEBUG
                if kind == 1 {
                    IrisCallAudioContinuityProbe.shared.event(.dispatchRejected)
                    if now() >= deadline { IrisCallAudioContinuityProbe.shared.event(.dispatchDeadlineRejected) }
                }
#endif
                return
            }
            dispatch(action)
        }
        guard pending.append(work, kind: kind) else { return }
        queue.async { [pending] in
            for work in pending.take() { work() }
        }
    }
}
