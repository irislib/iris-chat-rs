import AVFoundation

/// A few complete access units may overtake one another while FIPS repairs
/// fragments. Give a gap 50 ms, then discard dependent frames until an IDR.
final class IrisCallVideoReceiver {
    private struct Frame { let timestamp: UInt64; let key: Bool; let data: Data }
    private let queue: DispatchQueue
    private let decoder: IrisH264Decoder
    private let requestKeyFrame: () -> Void
    private var pending: [UInt32: Frame] = [:]
    private var expected: UInt32?
    private var gap: DispatchWorkItem?
    private var lastRequest: TimeInterval = 0

    init(queue: DispatchQueue, output: @escaping (CVPixelBuffer) -> Void, requestKeyFrame: @escaping () -> Void) {
        self.queue = queue
        self.decoder = IrisH264Decoder { pixel, _ in output(pixel) }
        self.requestKeyFrame = requestKeyFrame
        decoder.onFailure = { [weak self] in
            queue.async { [weak self] in
                self?.decoder.discontinuity()
                self?.requestRefresh()
            }
        }
    }

    func receive(sequence: UInt32, timestampUs: UInt64, keyFrame: Bool, data: Data) {
        if let expected, Int32(bitPattern: sequence &- expected) < 0 { return }
        guard pending[sequence] == nil else { return }
        pending[sequence] = Frame(timestamp: timestampUs, key: keyFrame, data: data)
        if expected == nil && keyFrame { expected = sequence }
        drain()
        if pending.count > 3 { expireGap() }
        if !pending.isEmpty && gap == nil {
            let timer = DispatchWorkItem { [weak self] in self?.expireGap() }
            gap = timer
            queue.asyncAfter(deadline: .now() + .milliseconds(50), execute: timer)
        }
    }

    private func drain() {
        while let expected, let frame = pending.removeValue(forKey: expected) {
            self.expected = expected &+ 1
            if !decoder.decode(frame.data, timestampUs: frame.timestamp, keyFrame: frame.key) { requestRefresh() }
        }
        if pending.isEmpty { gap?.cancel(); gap = nil }
    }

    private func expireGap() {
        gap?.cancel(); gap = nil
        guard !pending.isEmpty else { return }
        decoder.discontinuity()
        requestRefresh()
        let reference = expected ?? pending.keys.min()!
        let ordered = pending.keys.sorted { $0 &- reference < $1 &- reference }
        if let key = ordered.first(where: { pending[$0]?.key == true }) {
            pending = pending.filter { Int32(bitPattern: $0.key &- key) >= 0 }
            expected = key
            drain()
        } else {
            expected = ordered.last! &+ 1
            pending.removeAll(keepingCapacity: true)
        }
    }

    private func requestRefresh() {
        let now = ProcessInfo.processInfo.systemUptime
        if now - lastRequest >= 1 { lastRequest = now; requestKeyFrame() }
    }

    func stop() { gap?.cancel(); gap = nil; pending.removeAll(); expected = nil; decoder.stop() }
    deinit { gap?.cancel(); decoder.stop() }
}
