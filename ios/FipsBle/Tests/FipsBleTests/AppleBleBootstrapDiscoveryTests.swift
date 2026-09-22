import Foundation
@testable import FipsBle
import XCTest

final class AppleBleBootstrapDiscoveryTests: XCTestCase {
    private let peer = UUID()
    private let bootstrap = Data([1, 2, 3])

    func testRepeatedAdvertisementsReuseBootstrapWithoutAnotherGattRead() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        XCTAssertTrue(discovery.complete(peer, bootstrap: bootstrap))

        for _ in 0..<60 {
            XCTAssertEqual(discovery.begin(peer), .cached(bootstrap))
            XCTAssertFalse(discovery.isPending(peer))
        }
    }

    func testDuplicateAdvertisementsWhileReadingDoNotStartAnotherRead() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        for _ in 0..<60 {
            XCTAssertNil(discovery.begin(peer))
        }
        XCTAssertTrue(discovery.complete(peer, bootstrap: bootstrap))
    }

    func testDisconnectKeepsBootstrapForRediscoveryAndFipsReconnect() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        discovery.cancel(peer)

        XCTAssertEqual(discovery.begin(peer), .cached(bootstrap))
    }

    func testRejectedL2capRefreshesBootstrapForRestartedPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)

        XCTAssertEqual(discovery.begin(peer, refresh: true), .read)
        XCTAssertNil(discovery.begin(peer))
        let restartedBootstrap = Data([4, 5, 6])
        XCTAssertTrue(discovery.complete(peer, bootstrap: restartedBootstrap))
        XCTAssertEqual(discovery.begin(peer), .cached(restartedBootstrap))
    }

    func testFailedRefreshDoesNotReuseStaleBootstrap() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)

        XCTAssertEqual(discovery.begin(peer, refresh: true), .read)
        discovery.cancel(peer)
        XCTAssertEqual(discovery.begin(peer), .read)
    }

    func testStoppedScanIgnoresLateReadAndStartsFresh() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.reset()
        XCTAssertFalse(discovery.complete(peer, bootstrap: bootstrap))
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        discovery.reset()
        XCTAssertEqual(discovery.begin(peer), .read)
    }

    func testPendingLimitDoesNotBlockKnownPeers() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        for _ in 0..<64 {
            XCTAssertEqual(discovery.begin(UUID()), .read)
        }
        XCTAssertNil(discovery.begin(UUID()))
        XCTAssertEqual(discovery.begin(peer), .cached(bootstrap))
    }

    func testUnsolicitedValuesCannotPopulateCache() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertFalse(discovery.complete(peer, bootstrap: bootstrap))
        XCTAssertEqual(discovery.begin(peer), .read)
    }
}
