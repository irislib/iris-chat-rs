package to.iris.chat.push

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.debug.backgroundControlEnvironment

class BackgroundControlStartupTest {
    private fun bootstrap() = JSONObject().put("phase", "bootstrap").put("relay_url", "ws://127.0.0.1:1234")
    private fun paired() = bootstrap().put("phase", "paired")
        .put("peer_npub", "npub1" + "q".repeat(58)).put("peer_udp", "192.168.1.2:1234").put("local_udp_port", 45678)

    @Test fun bootstrapCannotEnableAnEndpointOrAlternateDiscovery() {
        val env = backgroundControlEnvironment(bootstrap())
        for (key in listOf("IRIS_FIPS_WEBSOCKET_SEED_URLS", "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR",
            "IRIS_CHAT_FIPS_ROUTED_PEERS", "IRIS_CHAT_FIPS_STATIC_PEERS", "IRIS_CHAT_FIPS_UDP_BIND_ADDR")) {
            assertEquals("", env[key])
        }
        assertEquals("0", env["IRIS_CHAT_FIPS_ENABLE_WEBRTC"])
        assertEquals("0", env["IRIS_CHAT_SAME_HOST_HASHTREE"])
        assertFalse(env.containsKey("IRIS_UPDATE_HTREE_REF"))
        assertTrue(runCatching { backgroundControlEnvironment(bootstrap().put("peer_npub", "unexpected")) }.isFailure)
    }

    @Test fun pairedSettingsUseTheSameDeviceForStaticRouteAndUpdatePublisher() {
        val config = paired()
        val env = backgroundControlEnvironment(config)
        val npub = config.getString("peer_npub")
        assertEquals("$npub=udp:192.168.1.2:1234", env["IRIS_CHAT_FIPS_STATIC_PEERS"])
        assertEquals("htree://$npub/diagnostic-control/latest", env["IRIS_UPDATE_HTREE_REF"])
        assertEquals("0.0.0.0:45678", env["IRIS_CHAT_FIPS_UDP_BIND_ADDR"])
    }

    @Test fun refusesPublicAddressesUnknownKeysAndMissingSettings() {
        for (config in listOf(paired().put("peer_udp", "8.8.8.8:1234"),
            paired().put("relay_url", "wss://example.com"), paired().put("local_udp_port", "45678"),
            paired().put("owner_nsec", "never-accepted"), bootstrap().put("phase", "unknown"))) {
            assertTrue(runCatching { backgroundControlEnvironment(config) }.isFailure)
        }
    }
}
