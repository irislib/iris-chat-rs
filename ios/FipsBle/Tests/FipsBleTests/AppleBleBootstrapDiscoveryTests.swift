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
        discovery.fail(peer)

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

        XCTAssertEqual(discovery.begin(peer, refresh: true, now: 10), .read)
        discovery.fail(peer, now: 10)
        XCTAssertNil(discovery.begin(peer, now: 14))
        XCTAssertEqual(discovery.begin(peer, now: 15), .read)
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

    func testFailedDiscoveryIgnoresRepeatedAdvertisementsUntilRetryIsDue() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        for _ in 0..<60 {
            XCTAssertNil(discovery.begin(peer, now: 11))
            XCTAssertNil(discovery.begin(peer, refresh: true, now: 12))
        }
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertEqual(discovery.begin(peer, now: 15), .read)
    }

    func testRetryDoesNotRequireAnotherAdvertisement() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        XCTAssertTrue(discovery.dueRetries(now: 14).isEmpty)
        XCTAssertEqual(discovery.dueRetries(now: 15), [peer])
        XCTAssertEqual(discovery.begin(peer, now: 15), .read)
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 100).isEmpty)
    }

    func testConsecutiveFailuresBackOffToOneMinute() {
        var discovery = AppleBleBootstrapDiscovery()
        var now: TimeInterval = 10
        for delay: TimeInterval in [5, 10, 20, 40, 60, 60, 60] {
            XCTAssertEqual(discovery.begin(peer, now: now), .read)
            discovery.fail(peer, now: now)
            XCTAssertEqual(discovery.nextRetryAt(), now + delay)
            XCTAssertNil(discovery.begin(peer, now: now + delay - 0.1))
            now += delay
        }
    }

    func testDisconnectAfterFailureDoesNotResetRetryDeadline() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        discovery.fail(peer, now: 11)
        XCTAssertEqual(discovery.nextRetryAt(), 15)
    }

    func testSuccessfulReadClearsFailureHistory() {
        var discovery = AppleBleBootstrapDiscovery()
        for now: TimeInterval in [10, 15] {
            XCTAssertEqual(discovery.begin(peer, now: now), .read)
            discovery.fail(peer, now: now)
        }
        XCTAssertEqual(discovery.begin(peer, now: 25), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertEqual(discovery.begin(peer, refresh: true, now: 30), .read)
        discovery.fail(peer, now: 30)
        XCTAssertEqual(discovery.nextRetryAt(), 35)
    }

    func testStoppedScanDropsRetriesAndIgnoresLateFailures() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        discovery.reset()
        discovery.fail(peer, now: 11)
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 100).isEmpty)
        XCTAssertEqual(discovery.begin(peer, now: 11), .read)
    }

    func testFullPendingQueueWaitsForACompletionBeforeSchedulingRetry() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        var pending: [UUID] = []
        for _ in 0..<64 {
            let identifier = UUID()
            pending.append(identifier)
            XCTAssertEqual(discovery.begin(identifier, now: 10), .read)
        }
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 20).isEmpty)
        discovery.complete(pending[0], bootstrap: bootstrap)
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertEqual(discovery.dueRetries(now: 20), [peer])
    }

    func testAudioPlaybackDefersNewDiscoveryWithoutLosingPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        for _ in 0..<60 {
            XCTAssertNil(discovery.begin(peer, canRead: false, now: 10))
        }
        XCTAssertFalse(discovery.isPending(peer))
        // The audio activity notification can resume discovery without a new ad.
        XCTAssertEqual(discovery.dueRetries(now: 20), [peer])
        XCTAssertEqual(discovery.begin(peer, now: 20), .read)
        discovery.fail(peer, now: 20)
        XCTAssertEqual(discovery.nextRetryAt(), 25)
    }

    func testAudioPlaybackStillAllowsCachedDiscovery() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        XCTAssertEqual(discovery.begin(peer, canRead: false), .cached(bootstrap))
        XCTAssertNil(discovery.nextRetryAt())
    }

    func testAudioPlaybackPreservesFailureBackoffAndRefreshInvalidation() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        XCTAssertNil(discovery.begin(peer, canRead: false, now: 11))
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertNil(discovery.begin(peer, now: 14))
        XCTAssertEqual(discovery.begin(peer, now: 15), .read)
        discovery.complete(peer, bootstrap: bootstrap)
        XCTAssertNil(discovery.begin(peer, refresh: true, canRead: false, now: 20))
        XCTAssertEqual(discovery.begin(peer, now: 25), .read)
    }

    func testDeferredOverflowPeerDoesNotBlockIdentifiedPeerRetriesDuringAudio() {
        var discovery = AppleBleBootstrapDiscovery()
        let ambiguousPeer = UUID()
        XCTAssertNil(discovery.begin(ambiguousPeer, canRead: false, now: 10))
        XCTAssertEqual(discovery.begin(peer, now: 10), .read)
        discovery.fail(peer, now: 10)
        let identified: (UUID) -> Bool = { $0 == self.peer }
        XCTAssertEqual(discovery.nextRetryAt(allowing: identified), 15)
        XCTAssertTrue(discovery.dueRetries(now: 14, allowing: identified).isEmpty)
        XCTAssertEqual(discovery.dueRetries(now: 15, allowing: identified), [peer])
        XCTAssertEqual(discovery.begin(peer, now: 15), .read)
        XCTAssertNil(discovery.nextRetryAt(allowing: identified))
        // Audio ending makes the otherwise suppressed peer eligible again.
        XCTAssertEqual(discovery.dueRetries(now: 20), [ambiguousPeer])
    }
}
