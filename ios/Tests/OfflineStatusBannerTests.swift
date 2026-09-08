#if os(iOS)
import XCTest
@testable import IrisChat

final class OfflineStatusBannerTests: XCTestCase {
    func testUnavailableServersDoNotClaimDeviceRadiosAreOff() {
        XCTAssertEqual(
            offlineStatusBannerText(
                networkStatus: offlineStatus(),
                isLoggedIn: true,
                appSceneIsActive: true,
                foregroundedAt: Date(timeIntervalSince1970: 0),
                now: Date(timeIntervalSince1970: 100)
            ),
            "Can’t reach message servers"
        )
    }

    func testLoggedOutNeverShowsWarningEvenAfterGracePeriod() {
        XCTAssertNil(text(isLoggedIn: false))
    }

    func testBackgroundedAppDoesNotShowWarning() {
        XCTAssertNil(text(appSceneIsActive: false))
    }

    func testWaitsForBothOfflineAndForegroundGracePeriods() {
        XCTAssertNil(text(now: 30))
        XCTAssertEqual(text(now: 31), "Can’t reach message servers")
        XCTAssertNil(text(foregroundedAt: 90, now: 119))
        XCTAssertEqual(text(foregroundedAt: 90, now: 120), "Can’t reach message servers")
    }

    func testUnknownAndOnlineStatesDoNotShowWarning() {
        XCTAssertNil(text(networkStatus: nil))
        var network = offlineStatus()
        network.relayUrls = []
        XCTAssertNil(text(networkStatus: network))
        network = offlineStatus()
        network.relayConnections = []
        XCTAssertNil(text(networkStatus: network))
        network = offlineStatus()
        network.connectedRelayCount = 1
        XCTAssertNil(text(networkStatus: network))
        network = offlineStatus()
        network.allRelaysOfflineSinceSecs = nil
        XCTAssertNil(text(networkStatus: network))
    }

    func testConnectingStatesDoNotShowWarning() {
        for status in ["connecting", "pending", "connected"] {
            var network = offlineStatus()
            network.relayConnections[0].status = status
            XCTAssertNil(text(networkStatus: network))
        }
    }

    func testBlockedServersStillShowWarning() {
        var network = offlineStatus()
        network.relayConnections[0].status = "blocked"
        XCTAssertEqual(text(networkStatus: network), "Can’t reach message servers")
    }

    private func text(
        isLoggedIn: Bool = true,
        appSceneIsActive: Bool = true,
        foregroundedAt: TimeInterval = 0,
        now: TimeInterval = 100
    ) -> String? {
        offlineStatusBannerText(
            networkStatus: offlineStatus(),
            isLoggedIn: isLoggedIn,
            appSceneIsActive: appSceneIsActive,
            foregroundedAt: Date(timeIntervalSince1970: foregroundedAt),
            now: Date(timeIntervalSince1970: now)
        )
    }

    private func text(networkStatus: NetworkStatusSnapshot?) -> String? {
        offlineStatusBannerText(
            networkStatus: networkStatus,
            isLoggedIn: true,
            appSceneIsActive: true,
            foregroundedAt: Date(timeIntervalSince1970: 0),
            now: Date(timeIntervalSince1970: 100)
        )
    }

    private func offlineStatus() -> NetworkStatusSnapshot {
        NetworkStatusSnapshot(
            relaySetId: "test",
            relayUrls: ["ws://127.0.0.1:9"],
            relayConnections: [RelayConnectionSnapshot(url: "ws://127.0.0.1:9", status: "offline")],
            connectedRelayCount: 0,
            allRelaysOfflineSinceSecs: 1,
            syncing: false,
            pendingOutboundCount: 0,
            pendingGroupControlCount: 0,
            recentEventCount: 0,
            recentLogCount: 0,
            lastDebugCategory: nil,
            lastDebugDetail: nil
        )
    }
}
#endif
