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
}
#endif
