package to.iris.chat.ui.navigation

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import to.iris.chat.rust.NetworkStatusSnapshot
import to.iris.chat.rust.RelayConnectionSnapshot

class OfflineStatusBannerTest {
    @Test
    fun loggedOutNeverShowsWarningEvenAfterGracePeriod() {
        assertNull(offlineStatusBannerText(offlineStatus(), false, 0, 100))
        assertNull(offlineBannerDeadlineSecs(offlineStatus(), false, 0))
    }

    @Test
    fun unavailableServersDoNotClaimDeviceRadiosAreOff() {
        assertEquals("Can’t reach message servers", offlineStatusBannerText(offlineStatus(), true, 0, 100))
    }

    @Test
    fun waitsForBothOfflineAndForegroundGracePeriods() {
        assertNull(offlineStatusBannerText(offlineStatus(), true, 0, 30))
        assertEquals("Can’t reach message servers", offlineStatusBannerText(offlineStatus(), true, 0, 31))
        assertNull(offlineStatusBannerText(offlineStatus(), true, 90, 119))
        assertEquals("Can’t reach message servers", offlineStatusBannerText(offlineStatus(), true, 90, 120))
    }

    @Test
    fun connectingUnknownAndOnlineStatesDoNotShowWarning() {
        assertNull(offlineStatusBannerText(null, true, 0, 100))
        assertNull(offlineStatusBannerText(offlineStatus().copy(relayUrls = emptyList()), true, 0, 100))
        assertNull(offlineStatusBannerText(offlineStatus().copy(relayConnections = emptyList()), true, 0, 100))
        assertNull(offlineStatusBannerText(offlineStatus().copy(connectedRelayCount = 1uL), true, 0, 100))
        assertNull(offlineStatusBannerText(offlineStatus().copy(allRelaysOfflineSinceSecs = null), true, 0, 100))
        for (status in listOf("connecting", "pending", "connected")) {
            val network = offlineStatus().copy(
                relayConnections = listOf(RelayConnectionSnapshot("ws://127.0.0.1:9", status)),
            )
            assertNull(offlineStatusBannerText(network, true, 0, 100))
        }
    }

    @Test
    fun blockedServersStillShowWarning() {
        val network = offlineStatus().copy(
            relayConnections = listOf(RelayConnectionSnapshot("ws://127.0.0.1:9", "blocked")),
        )
        assertEquals("Can’t reach message servers", offlineStatusBannerText(network, true, 0, 100))
    }

    private fun offlineStatus() = NetworkStatusSnapshot(
        relaySetId = "test",
        relayUrls = listOf("ws://127.0.0.1:9"),
        relayConnections = listOf(RelayConnectionSnapshot("ws://127.0.0.1:9", "offline")),
        connectedRelayCount = 0uL,
        allRelaysOfflineSinceSecs = 1uL,
        syncing = false,
        pendingOutboundCount = 0uL,
        pendingGroupControlCount = 0uL,
        recentEventCount = 0uL,
        recentLogCount = 0uL,
        lastDebugCategory = null,
        lastDebugDetail = null,
    )
}
