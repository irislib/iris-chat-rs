import Foundation
@testable import FipsBle
import XCTest

final class AppleBleBootstrapDiscoveryTests: XCTestCase {
    func testResolvedPeerAdvertisementsDoNotReopenDiscardedAlternateLink() {
        var discovery = AppleBleBootstrapDiscovery()
        let peer = UUID()
        XCTAssertTrue(discovery.begin(peer))
        XCTAssertTrue(discovery.complete(peer))
        // FIPS has identified the peer and may close this link because LAN is
        // already healthy. Its disconnect must not make each ad a new peer.
        discovery.fail(peer)
        for tick in 0..<120 {
            XCTAssertFalse(discovery.begin(peer, now: TimeInterval(tick)))
        }
        // A genuine service change still lets FIPS learn the new PSM.
        XCTAssertTrue(discovery.begin(peer, refresh: true))
    }

    private let peer = UUID()

    func testRepeatedAdvertisementsDoNotRepeatSuccessfulDiscovery() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        XCTAssertTrue(discovery.complete(peer))

        for _ in 0..<60 {
            XCTAssertFalse(discovery.begin(peer))
            XCTAssertFalse(discovery.isPending(peer))
        }
    }

    func testDuplicateAdvertisementsWhileReadingDoNotStartAnotherRead() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        for _ in 0..<60 {
            XCTAssertFalse(discovery.begin(peer))
        }
        XCTAssertTrue(discovery.complete(peer))
    }

    func testDisconnectDoesNotRediscoverResolvedPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)
        discovery.fail(peer)

        XCTAssertFalse(discovery.begin(peer))
    }

    func testRejectedL2capRefreshesBootstrapForRestartedPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)

        XCTAssertTrue(discovery.begin(peer, refresh: true))
        XCTAssertFalse(discovery.begin(peer))
        XCTAssertTrue(discovery.complete(peer))
        XCTAssertFalse(discovery.begin(peer))
    }

    func testFailedRefreshDoesNotReuseStaleBootstrap() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)

        XCTAssertTrue(discovery.begin(peer, refresh: true, now: 10))
        discovery.fail(peer, now: 10)
        XCTAssertFalse(discovery.begin(peer, now: 14))
        XCTAssertTrue(discovery.begin(peer, now: 15))
    }

    func testStoppedScanIgnoresLateReadAndStartsFresh() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.reset()
        XCTAssertFalse(discovery.complete(peer))
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)
        discovery.reset()
        XCTAssertTrue(discovery.begin(peer))
    }

    func testPendingLimitDoesNotRediscoverKnownPeers() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)
        for _ in 0..<64 {
            XCTAssertTrue(discovery.begin(UUID()))
        }
        XCTAssertFalse(discovery.begin(UUID()))
        XCTAssertFalse(discovery.begin(peer))
    }

    func testUnsolicitedValuesCannotResolvePeer() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertFalse(discovery.complete(peer))
        XCTAssertTrue(discovery.begin(peer))
    }

    func testFailedDiscoveryIgnoresRepeatedAdvertisementsUntilRetryIsDue() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        for _ in 0..<60 {
            XCTAssertFalse(discovery.begin(peer, now: 11))
            XCTAssertFalse(discovery.begin(peer, refresh: true, now: 12))
        }
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertTrue(discovery.begin(peer, now: 15))
    }

    func testRetryDoesNotRequireAnotherAdvertisement() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        XCTAssertTrue(discovery.dueRetries(now: 14).isEmpty)
        XCTAssertEqual(discovery.dueRetries(now: 15), [peer])
        XCTAssertTrue(discovery.begin(peer, now: 15))
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 100).isEmpty)
    }

    func testConsecutiveFailuresBackOffToOneMinute() {
        var discovery = AppleBleBootstrapDiscovery()
        var now: TimeInterval = 10
        for delay: TimeInterval in [5, 10, 20, 40, 60, 60, 60] {
            XCTAssertTrue(discovery.begin(peer, now: now))
            discovery.fail(peer, now: now)
            XCTAssertEqual(discovery.nextRetryAt(), now + delay)
            XCTAssertFalse(discovery.begin(peer, now: now + delay - 0.1))
            now += delay
        }
    }

    func testDisconnectAfterFailureDoesNotResetRetryDeadline() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        discovery.fail(peer, now: 11)
        XCTAssertEqual(discovery.nextRetryAt(), 15)
    }

    func testSuccessfulReadClearsFailureHistory() {
        var discovery = AppleBleBootstrapDiscovery()
        for now: TimeInterval in [10, 15] {
            XCTAssertTrue(discovery.begin(peer, now: now))
            discovery.fail(peer, now: now)
        }
        XCTAssertTrue(discovery.begin(peer, now: 25))
        discovery.complete(peer)
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.begin(peer, refresh: true, now: 30))
        discovery.fail(peer, now: 30)
        XCTAssertEqual(discovery.nextRetryAt(), 35)
    }

    func testStoppedScanDropsRetriesAndIgnoresLateFailures() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        discovery.reset()
        discovery.fail(peer, now: 11)
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 100).isEmpty)
        XCTAssertTrue(discovery.begin(peer, now: 11))
    }

    func testFullPendingQueueWaitsForACompletionBeforeSchedulingRetry() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        var pending: [UUID] = []
        for _ in 0..<64 {
            let identifier = UUID()
            pending.append(identifier)
            XCTAssertTrue(discovery.begin(identifier, now: 10))
        }
        XCTAssertNil(discovery.nextRetryAt())
        XCTAssertTrue(discovery.dueRetries(now: 20).isEmpty)
        discovery.complete(pending[0])
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertEqual(discovery.dueRetries(now: 20), [peer])
    }

    func testAudioPlaybackDefersNewDiscoveryWithoutLosingPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        for _ in 0..<60 {
            XCTAssertFalse(discovery.begin(peer, canRead: false, now: 10))
        }
        XCTAssertFalse(discovery.isPending(peer))
        // The audio activity notification can resume discovery without a new ad.
        XCTAssertEqual(discovery.dueRetries(now: 20), [peer])
        XCTAssertTrue(discovery.begin(peer, now: 20))
        discovery.fail(peer, now: 20)
        XCTAssertEqual(discovery.nextRetryAt(), 25)
    }

    func testAudioPlaybackDoesNotRediscoverResolvedPeer() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer))
        discovery.complete(peer)
        XCTAssertFalse(discovery.begin(peer, canRead: false))
        XCTAssertNil(discovery.nextRetryAt())
    }

    func testAudioPlaybackPreservesFailureBackoffAndRefreshInvalidation() {
        var discovery = AppleBleBootstrapDiscovery()
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        XCTAssertFalse(discovery.begin(peer, canRead: false, now: 11))
        XCTAssertEqual(discovery.nextRetryAt(), 15)
        XCTAssertFalse(discovery.begin(peer, now: 14))
        XCTAssertTrue(discovery.begin(peer, now: 15))
        discovery.complete(peer)
        XCTAssertFalse(discovery.begin(peer, refresh: true, canRead: false, now: 20))
        XCTAssertTrue(discovery.begin(peer, now: 25))
    }

    func testDeferredOverflowPeerDoesNotBlockIdentifiedPeerRetriesDuringAudio() {
        var discovery = AppleBleBootstrapDiscovery()
        let ambiguousPeer = UUID()
        XCTAssertFalse(discovery.begin(ambiguousPeer, canRead: false, now: 10))
        XCTAssertTrue(discovery.begin(peer, now: 10))
        discovery.fail(peer, now: 10)
        let identified: (UUID) -> Bool = { $0 == self.peer }
        XCTAssertEqual(discovery.nextRetryAt(allowing: identified), 15)
        XCTAssertTrue(discovery.dueRetries(now: 14, allowing: identified).isEmpty)
        XCTAssertEqual(discovery.dueRetries(now: 15, allowing: identified), [peer])
        XCTAssertTrue(discovery.begin(peer, now: 15))
        XCTAssertNil(discovery.nextRetryAt(allowing: identified))
        // Audio ending makes the otherwise suppressed peer eligible again.
        XCTAssertEqual(discovery.dueRetries(now: 20), [ambiguousPeer])
    }
}
