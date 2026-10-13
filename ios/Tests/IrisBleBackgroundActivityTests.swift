#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class IrisBleBackgroundActivityTests: XCTestCase {
    @MainActor
    func testBluetoothWakeKeepsAllowanceUntilStorageBarrierAndDoesNotRenewOnWrites() async {
        var transitions: [String] = []
        var completions: [@Sendable () -> Void] = []
        var ended: [UIBackgroundTaskIdentifier] = []
        var starts = 0
        var deliveries = 0
        let activity = IrisBleBackgroundActivity(budget: 60, isBackground: { true }, begin: { _ in
            starts += 1
            return UIBackgroundTaskIdentifier(rawValue: starts)
        }, end: { ended.append($0) }) { generation, active, completion in
            transitions.append("\(generation):\(active)")
            if !active { completions.append(completion) }
        }
        activity.receive(wakesApp: false) { deliveries += 1 }
        XCTAssertEqual(starts, 0, "Outgoing writes cannot buy background time")
        activity.receive(wakesApp: true) { deliveries += 1 }
        for _ in 0..<20 { activity.receive(wakesApp: true) { deliveries += 1 } }
        XCTAssertEqual(starts, 1, "Packets inside a wake cannot extend its fixed deadline")
        activity.finish()
        XCTAssertEqual(transitions, ["1:true", "1:false"])
        XCTAssertTrue(ended.isEmpty, "iOS must wait for the storage acknowledgement")
        activity.receive(wakesApp: true) { deliveries += 1 }
        XCTAssertEqual(starts, 2, "A new actual callback can wake after a completed budget")
        completions[0]()
        await Task.yield()
        XCTAssertEqual(ended, [UIBackgroundTaskIdentifier(rawValue: 1)])
        activity.finish(generation: 1)
        XCTAssertEqual(transitions.last, "2:true", "Stale expiration cannot finish a newer wake")
        activity.close()
        completions[1]()
        await Task.yield()
        XCTAssertEqual(ended.count, 2)
        let count = deliveries
        activity.receive(wakesApp: true) { deliveries += 1 }
        XCTAssertEqual(deliveries, count)
    }

    @MainActor
    func testExpirationUsesStorageBarrierAndForegroundDoesNotRequestTime() async {
        var inBackground = false
        var expire: (@Sendable () -> Void)?
        var complete: (@Sendable () -> Void)?
        var ended = false
        var transitions: [Bool] = []
        let activity = IrisBleBackgroundActivity(budget: 60, isBackground: { inBackground }, begin: {
            expire = $0
            return UIBackgroundTaskIdentifier(rawValue: 42)
        }, end: { _ in ended = true }) { _, active, done in
            transitions.append(active)
            if !active { complete = done }
        }
        activity.receive(wakesApp: true) {}
        XCTAssertNil(expire)
        inBackground = true
        activity.receive(wakesApp: true) {}
        expire?()
        await Task.yield()
        XCTAssertEqual(transitions, [true, false])
        XCTAssertFalse(ended)
        complete?()
        await Task.yield()
        XCTAssertTrue(ended)
        activity.close()
    }

    @MainActor
    func testDeniedAllowanceDoesNotOpenCoreStorageGate() {
        var delivered = false
        let activity = IrisBleBackgroundActivity(isBackground: { true }, begin: { _ in .invalid },
            end: { _ in XCTFail("No task to end") }) { _, _, _ in XCTFail("No OS allowance") }
        activity.receive(wakesApp: true) { delivered = true }
        XCTAssertTrue(delivered, "Transport can buffer; the core remains suspended")
        activity.close()
    }
}
#endif
