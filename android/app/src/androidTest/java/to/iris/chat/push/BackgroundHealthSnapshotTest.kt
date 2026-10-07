package to.iris.chat.push

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.debug.filterBackgroundHealth

class BackgroundHealthSnapshotTest {
    private fun bundle() = JSONObject("""{
        "ffi_queue":{"core_support_bundle_timed_out":false},
        "relay_urls":["ws://127.0.0.1:1234"],
        "local_owner_pubkey_hex":"private-identity",
        "relay_transport":{
            "phase":"connected", "connected_relay_count":1, "pending_relay_publish_count":3,
            "retry_backoff_attempt":0, "next_retry_due_in_ms":null,
            "connect_in_flight":false, "connect_dirty":false, "force_reconnect_dirty":false,
            "publish_drain_in_flight":false, "publish_drain_dirty":false,
            "last_connect_reason":"private-reason"
        }
    }""")

    @Test fun keepsOnlyAggregatesAndChecksActualConfiguredRelay() {
        val snapshot = filterBackgroundHealth(bundle(), "ws://127.0.0.1:1234")
        assertTrue(snapshot.getBoolean("expected_relay_matches"))
        assertEquals(3, snapshot.getInt("pending_relay_publish_count"))
        assertFalse(snapshot.toString().contains("private"))
        assertFalse(snapshot.toString().contains("ws://"))
        assertFalse(filterBackgroundHealth(bundle(), "ws://127.0.0.1:5678")
            .getBoolean("expected_relay_matches"))
    }

    @Test fun rejectsTimeoutAndMissingOrCoercedCounts() {
        val timedOut = bundle().apply { getJSONObject("ffi_queue").put("core_support_bundle_timed_out", true) }
        val missing = bundle().apply { getJSONObject("relay_transport").remove("pending_relay_publish_count") }
        val coerced = bundle().apply { getJSONObject("relay_transport").put("connected_relay_count", "1") }
        val negative = bundle().apply { getJSONObject("relay_transport").put("pending_relay_publish_count", -1) }
        for (invalid in listOf(timedOut, missing, coerced, negative)) {
            assertTrue(runCatching { filterBackgroundHealth(invalid, "ws://127.0.0.1:1234") }.isFailure)
        }
    }
}
