import Foundation

/// Hands complete capture callbacks to the media queue without blocking audio I/O.
final class IrisCallAudioCaptureQueue {
    private let queue: DispatchQueue
    private let slots = DispatchSemaphore(value: 2)
    private let now: () -> UInt64

    init(queue: DispatchQueue, now: @escaping () -> UInt64 = { DispatchTime.now().uptimeNanoseconds }) {
        self.queue = queue
        self.now = now
    }

    func submit(_ frames: [[Int16]], timestampUs: UInt64,
                consume: @escaping ([Int16], UInt64) -> Void) {
        // AVAudioNode taps can deliver 100–400 ms despite a smaller requested
        // size. Reserve a slot for the whole callback, not each Opus frame.
        // Bound both memory and queue age if the media thread is overloaded.
        guard !frames.isEmpty, frames.count <= 24,
              slots.wait(timeout: .now()) == .success else { return }
        let enqueued = now()
        queue.async { [slots, now] in
            defer { slots.signal() }
            guard now() &- enqueued <= 100_000_000 else { return }
            for (index, frame) in frames.enumerated() {
                consume(frame, timestampUs + UInt64(index) * 20_000)
            }
        }
    }
}

/// Serial-queue owner of a bounded set of scheduled device-output buffers.
final class IrisCallAudioPlayoutQueue {
    private let queue: DispatchQueue
    private let schedule: (@escaping () -> Void) -> Bool
    private var queued = 0
    private var generation: UInt64 = 0
    private var active = false

    init(queue: DispatchQueue, schedule: @escaping (@escaping () -> Void) -> Bool) {
        self.queue = queue
        self.schedule = schedule
    }

    func start() { active = true; refill() }

    private func refill() {
        // Device consumption supplies the clock. A delayed timer must not leave
        // holes in speech or steadily accumulate latency relative to capture.
        while active && queued < 3 {
            queued += 1
            let generation = self.generation
            if !schedule({ [weak self] in
                self?.queue.async { [weak self] in
                    guard let self, self.active, self.generation == generation else { return }
                    self.queued -= 1
                    self.refill()
                }
            }) { queued -= 1; break }
        }
    }

    func stop() { active = false; generation &+= 1; queued = 0 }
}
