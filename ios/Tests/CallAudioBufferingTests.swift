import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallAudioBufferingTests: XCTestCase {
    func testEveryFrameIn100msMicrophoneCallbackIsDeliveredInOrder() {
        let queue = DispatchQueue(label: "test.audio.capture")
        let capture = IrisCallAudioCaptureQueue(queue: queue)
        let frames = (0..<5).map { [Int16](repeating: Int16($0 + 1), count: 960) }
        var captured: [[Int16]] = []
        var timestamps: [UInt64] = []
        queue.suspend()
        capture.submit(frames, timestampUs: 1_000_000) { frame, timestamp in
            captured.append(frame)
            timestamps.append(timestamp)
        }
        queue.resume()
        queue.sync {}
        XCTAssertEqual(captured, frames)
        XCTAssertEqual(timestamps, [1_000_000, 1_020_000, 1_040_000, 1_060_000, 1_080_000])
    }

    func testEveryFrameInMaximum400msMicrophoneCallbackIsDelivered() {
        let queue = DispatchQueue(label: "test.audio.capture.large")
        let capture = IrisCallAudioCaptureQueue(queue: queue)
        let frames = (0..<20).map { [Int16](repeating: Int16($0), count: 960) }
        var captured: [[Int16]] = []
        queue.suspend()
        capture.submit(frames, timestampUs: 0) { frame, _ in captured.append(frame) }
        queue.resume()
        queue.sync {}
        XCTAssertEqual(captured, frames)
    }

    func testDeviceConsumptionRefillsPlayoutWithoutTimerTicks() {
        let queue = DispatchQueue(label: "test.audio.playout")
        var completions: [() -> Void] = []
        let playout = IrisCallAudioPlayoutQueue(queue: queue) { done in
            completions.append(done)
            return true
        }
        queue.sync { playout.start() }
        XCTAssertEqual(completions.count, 3, "prime enough output to tolerate scheduler jitter")
        // Deliver all device callbacks together after a delayed media-queue wakeup.
        let consumed = completions
        consumed.forEach { $0() }
        queue.sync {}
        XCTAssertEqual(completions.count, consumed.count + 3, "refill consumed audio without wall-clock pacing")
        queue.sync { playout.stop() }
    }

    func testCaptureBacklogIsBoundedAndStaleCallbacksAreDiscarded() {
        let queue = DispatchQueue(label: "test.audio.capture.overload")
        var now: UInt64 = 0
        let capture = IrisCallAudioCaptureQueue(queue: queue, now: { now })
        let frames = (0..<5).map { [Int16](repeating: Int16($0), count: 960) }
        var captured = 0
        queue.suspend()
        for _ in 0..<100 { capture.submit(frames, timestampUs: 0) { _, _ in captured += 1 } }
        queue.resume()
        queue.sync {}
        XCTAssertEqual(captured, 10, "at most two complete callbacks may be pending")
        captured = 0
        queue.suspend()
        capture.submit(frames, timestampUs: 0) { _, _ in captured += 1 }
        now = 101_000_000
        queue.resume()
        queue.sync {}
        XCTAssertEqual(captured, 0, "a stalled media queue must not send stale speech")
        capture.submit(frames, timestampUs: 0) { _, _ in captured += 1 }
        queue.sync {}
        XCTAssertEqual(captured, 5, "discarding a stale callback must release its slot")
    }

    func testFailedPlaybackSchedulingDoesNotSpinOrLeavePhantomBuffers() {
        let queue = DispatchQueue(label: "test.audio.playout.failed")
        var attempts = 0
        var available = false
        let playout = IrisCallAudioPlayoutQueue(queue: queue) { _ in attempts += 1; return available }
        queue.sync { playout.start() }
        XCTAssertEqual(attempts, 1)
        available = true
        queue.sync { playout.start() }
        XCTAssertEqual(attempts, 4)
        queue.sync { playout.stop() }
    }

    func testOldPlaybackCompletionsCannotRefillAfterStopOrRouteRestart() {
        let queue = DispatchQueue(label: "test.audio.playout.restart")
        var completions: [() -> Void] = []
        let playout = IrisCallAudioPlayoutQueue(queue: queue) { done in completions.append(done); return true }
        queue.sync { playout.start() }
        let stale = completions
        queue.sync { playout.stop(); playout.start() }
        let count = completions.count
        stale.forEach { $0() }
        queue.sync {}
        XCTAssertEqual(completions.count, count)
        queue.sync { playout.stop() }
        completions.forEach { $0() }
        queue.sync {}
        XCTAssertEqual(completions.count, count)
    }
}
