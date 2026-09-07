#if os(iOS)
import XCTest
@testable import IrisChat

@MainActor
final class MobilePushTokenCenterTests: XCTestCase {
    func testMissingTokenTimesOutAndLaterRegistrationStillWorks() async {
        let center = MobilePushTokenCenter()
        let finished = expectation(description: "missing APNs token times out")
        let task = Task {
            let result = await center.waitForApnsToken(timeoutNanoseconds: 10_000_000)
            XCTAssertNil(result)
            finished.fulfill()
        }
        await fulfillment(of: [finished], timeout: 1)
        // Also releases the old implementation's stuck waiter after a failure.
        center.setApnsToken("later-token")
        await task.value
        let token = await center.waitForApnsToken(timeoutNanoseconds: 10_000_000)
        XCTAssertEqual(token, "later-token")
    }

    func testCancellationFinishesWithoutWaitingForRegistration() async {
        let center = MobilePushTokenCenter()
        let finished = expectation(description: "cancelled token wait finishes")
        let task = Task {
            let result = await center.waitForApnsToken(timeoutNanoseconds: 60_000_000_000)
            XCTAssertNil(result)
            finished.fulfill()
        }
        await Task.yield()
        task.cancel()
        await fulfillment(of: [finished], timeout: 1)
        center.setApnsToken("cleanup-token")
        await task.value
    }

    func testRegistrationFailureFinishesAllCurrentWaiters() async {
        let center = MobilePushTokenCenter()
        let finished = expectation(description: "registration failure finishes waiters")
        finished.expectedFulfillmentCount = 2
        let tasks = (0..<2).map { _ in
            Task {
                let result = await center.waitForApnsToken(timeoutNanoseconds: 60_000_000_000)
                XCTAssertNil(result)
                finished.fulfill()
            }
        }
        await Task.yield()
        center.setApnsToken(nil)
        await fulfillment(of: [finished], timeout: 1)
        center.setApnsToken("cleanup-token")
        for task in tasks { await task.value }
    }
}
#endif
