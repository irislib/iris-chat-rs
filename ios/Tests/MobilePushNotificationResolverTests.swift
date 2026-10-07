#if os(iOS)
import XCTest
import UIKit
@testable import IrisChat

final class MobilePushNotificationResolverTests: XCTestCase {
    @MainActor
    func testPreviewKeepsBackgroundTimeUntilDatabaseReadFinishes() async {
        let started = expectation(description: "preview reading database")
        let gate = DispatchSemaphore(value: 0)
        var ended = 0
        let resolver = MobilePushNotificationResolver { _, _, payload in
            XCTAssertFalse(Thread.isMainThread)
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
            return resolveMobilePushNotificationPayload(rawPayloadJson: payload)
        }
        resolver.beginBackgroundTask = {
            IrisSuspendBackgroundTask(begin: { _ in .init(rawValue: 123) }, end: { _ in ended += 1 })
        }
        let preview = Task { await resolver.resolve(dataDir: "test", bundle: nil, payloadJson: "{}") }
        await fulfillment(of: [started], timeout: 2)
        XCTAssertEqual(ended, 0)
        gate.signal()
        let result = await preview.value
        XCTAssertNotNil(result)
        XCTAssertEqual(ended, 1)
    }

    @MainActor
    func testDeniedBackgroundTimeDoesNotOpenDatabase() async {
        let resolver = MobilePushNotificationResolver { _, _, _ in
            XCTFail("database must not open without background protection")
            return resolveMobilePushNotificationPayload(rawPayloadJson: "{}")
        }
        resolver.beginBackgroundTask = {
            IrisSuspendBackgroundTask(begin: { _ in .invalid }, end: { _ in XCTFail("invalid allowance") })
        }
        let result = await resolver.resolve(dataDir: "test", bundle: nil, payloadJson: "{}")
        XCTAssertNil(result)
    }

    @MainActor
    func testExpiredPreviewDoesNotNavigateAfterDatabaseReadReturns() async {
        let started = expectation(description: "preview reading database")
        let expired = expectation(description: "allowance expired")
        let gate = DispatchSemaphore(value: 0)
        var expire: (@Sendable () -> Void)?
        let resolver = MobilePushNotificationResolver { _, _, payload in
            started.fulfill()
            XCTAssertEqual(gate.wait(timeout: .now() + 5), .success)
            return resolveMobilePushNotificationPayload(rawPayloadJson: payload)
        }
        resolver.beginBackgroundTask = {
            IrisSuspendBackgroundTask(begin: {
                expire = $0
                return .init(rawValue: 123)
            }, end: { _ in expired.fulfill() })
        }
        let preview = Task { await resolver.resolve(dataDir: "test", bundle: nil, payloadJson: "{}") }
        await fulfillment(of: [started], timeout: 2)
        expire?()
        await fulfillment(of: [expired], timeout: 2)
        gate.signal()
        let result = await preview.value
        XCTAssertNil(result)
    }
}
#endif
