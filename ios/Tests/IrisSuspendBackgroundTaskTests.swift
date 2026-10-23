#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class IrisSuspendBackgroundTaskTests: XCTestCase {
    @MainActor
    func testAllowanceStaysOpenUntilWorkFinishes() {
        var ended: [UIBackgroundTaskIdentifier] = []
        let identifier = UIBackgroundTaskIdentifier(rawValue: 123)
        let task = IrisSuspendBackgroundTask(begin: { _ in identifier }, end: { ended.append($0) })
        XCTAssertTrue(ended.isEmpty)
        task.finish()
        task.finish()
        XCTAssertEqual(ended, [identifier])
    }

    @MainActor
    func testExpirationEndsAllowanceAndLateCompletionDoesNotEndItAgain() async {
        var expire: (@Sendable () -> Void)?
        var ended: [UIBackgroundTaskIdentifier] = []
        let identifier = UIBackgroundTaskIdentifier(rawValue: 456)
        let expired = expectation(description: "expired allowance released")
        let task = IrisSuspendBackgroundTask(begin: { expiration in
            expire = expiration
            return identifier
        }, end: {
            ended.append($0)
            expired.fulfill()
        })
        expire?()
        await fulfillment(of: [expired], timeout: 2)
        task.finish()
        XCTAssertEqual(ended, [identifier])
    }

    @MainActor
    func testDeniedAllowanceDoesNotEndAnInvalidIdentifier() {
        var ended = false
        let task = IrisSuspendBackgroundTask(begin: { _ in .invalid }, end: { _ in ended = true })
        task.finish()
        XCTAssertFalse(ended)
    }

    @MainActor
    func testExpirationCancelsCapturedStartupEpochBeforeReturning() async {
        let rust = MockRustApp()
        let cancellation = IrisStartupRestoreCancellation()
        cancellation.bind(to: rust)
        var expire: (@Sendable () -> Void)?
        var ends = 0
        let ended = expectation(description: "expired allowance ended")
        let task = IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
            expire = $0
            return .init(rawValue: 710)
        }, end: { _ in ends += 1; ended.fulfill() })
        expire?()
        XCTAssertEqual(rust.acceptedStartupRestoreCancellationEpochs, [0],
                       "cancellation must not wait for the MainActor finish task")
        await fulfillment(of: [ended], timeout: 2)
        task.finish()
        expire?()
        XCTAssertEqual(rust.startupRestoreCancellationEpochs, [0])
        XCTAssertEqual(ends, 1)
    }

    @MainActor
    func testExpirationBeforeBindingSurvivesAllowanceEnding() async {
        let rust = MockRustApp()
        let cancellation = IrisStartupRestoreCancellation()
        var expire: (@Sendable () -> Void)?
        let ended = expectation(description: "unbound allowance ended")
        let task = IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
            expire = $0
            return .init(rawValue: 711)
        }, end: { _ in ended.fulfill() })
        expire?()
        await fulfillment(of: [ended], timeout: 2)
        XCTAssertFalse(task.isActive)
        XCTAssertTrue(rust.startupRestoreCancellationEpochs.isEmpty)
        cancellation.bind(to: rust)
        cancellation.bind(to: rust)
        XCTAssertEqual(rust.acceptedStartupRestoreCancellationEpochs, [0])
    }

    @MainActor
    func testCompletedAllowanceDisarmsLateExpiration() {
        let rust = MockRustApp()
        let cancellation = IrisStartupRestoreCancellation()
        cancellation.bind(to: rust)
        var expire: (@Sendable () -> Void)?
        var ends = 0
        let task = IrisSuspendBackgroundTask(startupCancellation: cancellation, begin: {
            expire = $0
            return .init(rawValue: 712)
        }, end: { _ in ends += 1 })
        task.finish()
        expire?()
        XCTAssertTrue(rust.startupRestoreCancellationEpochs.isEmpty)
        XCTAssertEqual(ends, 1)
    }

    @MainActor
    func testForegroundInvalidatesCapturedExpirationToken() throws {
        let rust = MockRustApp()
        let cancellation = IrisStartupRestoreCancellation()
        cancellation.bind(to: rust)
        try rust.dispatch(action: .appForegrounded)
        cancellation.expire()
        XCTAssertEqual(rust.startupRestoreCancellationEpochs, [0],
                       "expiration must retain the old epoch instead of sampling a new one")
        XCTAssertTrue(rust.acceptedStartupRestoreCancellationEpochs.isEmpty)
    }

    @MainActor
    func testNormalCompletionBeforeBindingDisarmsCancellation() {
        let rust = MockRustApp()
        let cancellation = IrisStartupRestoreCancellation()
        cancellation.finish()
        cancellation.bind(to: rust)
        cancellation.expire()
        XCTAssertTrue(rust.startupRestoreCancellationEpochs.isEmpty)
    }
}
#endif
