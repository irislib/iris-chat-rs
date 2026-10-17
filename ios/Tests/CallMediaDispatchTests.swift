import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallMediaTestClock {
    private let lock = NSLock()
    private var time: TimeInterval = 0
    func now() -> TimeInterval { lock.lock(); defer { lock.unlock() }; return time }
    func set(_ time: TimeInterval) { lock.lock(); self.time = time; lock.unlock() }
}

final class CallMediaDispatchTests: XCTestCase {
    func testCoreHandoffPreservesWholeMicrophoneCallbacksWhileConsumerIsHeld() {
        for frameCount in [5, 20] {
            let queue = DispatchQueue(label: "test.call.send.held")
            let sender = IrisCallMediaDispatch(queue: queue, now: { 0 })
            var timestamps: [UInt64] = []
            queue.suspend()
            for frame in 0..<frameCount {
                sender.send(.sendCallMedia(callId: "call", kind: 1, timestampUs: UInt64(frame) * 20_000,
                                           keyFrame: false, data: Data([1])), allowed: { true }) { action in
                    if case .sendCallMedia(_, _, let timestamp, _, _) = action { timestamps.append(timestamp) }
                }
            }
            queue.resume()
            queue.sync {}
            XCTAssertEqual(timestamps, (0..<frameCount).map { UInt64($0) * 20_000 })
        }
    }
    func testBoundedOverloadRetainsEarliestAudioAndVideoIndependently() {
        let queue = DispatchQueue(label: "test.call.send.overload")
        let sender = IrisCallMediaDispatch(queue: queue, now: { 0 })
        var delivered: [UInt8: [UInt64]] = [:]
        queue.suspend()
        for kind: UInt8 in [2, 1] {
            for index in 0..<1_000 {
                sender.send(.sendCallMedia(callId: "call", kind: kind, timestampUs: UInt64(index),
                                           keyFrame: false, data: Data([1])), allowed: { true }) { action in
                    if case .sendCallMedia(_, let kind, let timestamp, _, _) = action {
                        delivered[kind, default: []].append(timestamp)
                    }
                }
            }
        }
        queue.resume()
        queue.sync {}
        XCTAssertEqual(delivered[1], Array(0..<24))
        XCTAssertEqual(delivered[2], Array(0..<4), "video cannot consume microphone batch capacity")
    }

    func testStaleAudioExpiresAndFreshAudioRecovers() {
        let queue = DispatchQueue(label: "test.call.send.age")
        let clock = CallMediaTestClock()
        let sender = IrisCallMediaDispatch(queue: queue, now: clock.now)
        var delivered = 0
        let action = AppAction.sendCallMedia(callId: "call", kind: 1, timestampUs: 0, keyFrame: false, data: Data([1]))
        queue.suspend()
        sender.send(action, allowed: { true }) { _ in delivered += 1 }
        clock.set(0.101)
        queue.resume()
        queue.sync {}
        XCTAssertEqual(delivered, 0)
        sender.send(action, allowed: { true }) { _ in delivered += 1 }
        queue.sync {}
        XCTAssertEqual(delivered, 1)
    }

    func testReplacingCoreInvalidatesOldRunnerEvenWhenCallIDsMatch() {
        let queue = DispatchQueue(label: "test.call.send.core")
        let sender = IrisCallMediaDispatch(queue: queue, now: { 0 })
        let action = AppAction.sendCallMedia(callId: "call", kind: 1, timestampUs: 0, keyFrame: false, data: Data([1]))
        var runners: [String] = []
        queue.suspend()
        sender.send(action, allowed: { true }) { _ in runners.append("old") }
        sender.invalidate()
        sender.send(action, allowed: { true }) { _ in runners.append("new") }
        queue.resume()
        queue.sync {}
        XCTAssertEqual(runners, ["new"])
    }

}
