#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class ReadNotificationCleanupTests: XCTestCase {
    private final class LookupActivity: @unchecked Sendable {
        private let lock = NSLock()
        private var calls = 0
        private var active = 0
        private var maximum = 0

        func begin() -> Bool {
            lock.lock()
            defer { lock.unlock() }
            calls += 1
            active += 1
            maximum = max(maximum, active)
            return calls == 1
        }

        func end() {
            lock.lock()
            defer { lock.unlock() }
            active -= 1
        }

        var snapshot: (calls: Int, maximum: Int) {
            lock.lock()
            defer { lock.unlock() }
            return (calls, maximum)
        }
    }

    private let bundle = StoredAccountBundle(ownerNsec: nil, ownerPubkeyHex: "owner", deviceNsec: "device")

    @MainActor
    func testPushAndScheduledCleanupShareOneWorkerAndKeepBackgroundTimeUntilItReturns() async {
        let started = expectation(description: "first lookup started")
        let finished = expectation(description: "push callback completed")
        let gate = DispatchSemaphore(value: 0)
        let activity = LookupActivity()
        var ended = 0
        let cleanup = ReadNotificationCleanup(
            delivered: { [("notification", "payload")] },
            resolve: { _, _, _ in
                XCTAssertFalse(Thread.isMainThread)
                let first = activity.begin()
                defer { activity.end() }
                if first {
                    started.fulfill()
                    XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
                }
                return [0]
            },
            remove: { _ in },
            beginBackgroundTask: {
                IrisSuspendBackgroundTask(begin: { _ in .init(rawValue: 123) }, end: { _ in ended += 1 })
            }
        )
        cleanup.schedule(dataDir: "test", bundle: bundle)
        await fulfillment(of: [started], timeout: 2)
        for _ in 0..<6 {
            cleanup.schedule(dataDir: "test", bundle: bundle)
        }
        Task { @MainActor in
            await cleanup.dismissRead(dataDir: "test", bundle: bundle)
            finished.fulfill()
        }
        await Task.yield()
        XCTAssertEqual(ended, 0)
        gate.signal()
        await fulfillment(of: [finished], timeout: 3)
        XCTAssertEqual(activity.snapshot.maximum, 1)
        XCTAssertLessThanOrEqual(activity.snapshot.calls, 3)
        XCTAssertGreaterThan(ended, 0)
    }

    @MainActor
    func testAccountChangeWaitsForOldWorkerAndDiscardsItsResult() async {
        let started = expectation(description: "old account lookup started")
        let gate = DispatchSemaphore(value: 0)
        var removed: [[String]] = []
        let cleanup = ReadNotificationCleanup(
            delivered: { [("notification", "payload")] },
            resolve: { _, bundle, _ in
                if bundle.ownerPubkeyHex == "owner" {
                    started.fulfill()
                    XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
                    return [0]
                }
                return []
            },
            remove: { removed.append($0) },
            beginBackgroundTask: {
                IrisSuspendBackgroundTask(begin: { _ in .init(rawValue: 123) }, end: { _ in })
            }
        )
        cleanup.schedule(dataDir: "test", bundle: bundle)
        await fulfillment(of: [started], timeout: 2)
        let next = StoredAccountBundle(ownerNsec: nil, ownerPubkeyHex: "next", deviceNsec: "next-device")
        cleanup.schedule(dataDir: "next", bundle: next)
        gate.signal()
        await cleanup.dismissRead(dataDir: "next", bundle: next)
        XCTAssertFalse(removed.contains(["notification"]))
    }

    @MainActor
    func testExpiredBackgroundTimeDoesNotStartDatabaseWorkAfterFetchingNotifications() async {
        let fetching = expectation(description: "fetching notifications")
        let expired = expectation(description: "allowance expired")
        var resume: CheckedContinuation<[(String, String)], Never>?
        var expire: (@Sendable () -> Void)?
        let cleanup = ReadNotificationCleanup(
            delivered: {
                await withCheckedContinuation {
                    resume = $0
                    fetching.fulfill()
                }
            },
            resolve: { _, _, _ in XCTFail("database work after expiration"); return [] },
            remove: { _ in XCTFail("unexpected removal") },
            beginBackgroundTask: {
                IrisSuspendBackgroundTask(begin: { expiration in
                    expire = expiration
                    return .init(rawValue: 123)
                }, end: { _ in expired.fulfill() })
            }
        )
        let waiting = Task { await cleanup.dismissRead(dataDir: "test", bundle: bundle) }
        await fulfillment(of: [fetching], timeout: 2)
        expire?()
        await fulfillment(of: [expired], timeout: 2)
        resume?.resume(returning: [("notification", "payload")])
        await waiting.value
    }

    @MainActor
    func testDeniedBackgroundTimeDoesNotStartDatabaseWork() async {
        let cleanup = ReadNotificationCleanup(
            delivered: { [("notification", "payload")] },
            resolve: { _, _, _ in XCTFail("database work without background protection"); return [] },
            remove: { _ in XCTFail("unexpected removal") },
            beginBackgroundTask: {
                IrisSuspendBackgroundTask(begin: { _ in .invalid }, end: { _ in XCTFail("invalid allowance") })
            }
        )
        await cleanup.dismissRead(dataDir: "test", bundle: bundle)
    }
}
#endif
