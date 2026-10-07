package to.iris.chat.push

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.debug.filterBackgroundFipsHealth

class BackgroundFipsHealthSnapshotTest {
    private fun sample() = JSONObject("""{
        "valid":true,"status":"available","scope":"connected_authenticated_peers","sample_id":"process:2",
        "connected_peer_count":1,"configured_direct_peer_count":1,
        "connected_configured_direct_peer_count":1,"unexpected_connected_peer_count":0,
        "transports":{"udp":{"connected_peer_count":1,"rx_packets":3,"tx_packets":5,"rx_bytes":150,"tx_bytes":275}},
        "interval":{"valid":true,"reason":"comparable","since_sample_id":"process:1","elapsed_ms":130000,
            "transport_deltas":{"udp":{"rx_packets":1,"tx_packets":2,"rx_bytes":50,"tx_bytes":75}}}
    }""")

    @Test fun filtersPrivateFieldsAtEveryDepthAndPreservesExactIds() {
        val input = sample()
        val interval = input.getJSONObject("interval")
        for (row in listOf(input, input.getJSONObject("transports").getJSONObject("udp"),
            interval, interval.getJSONObject("transport_deltas").getJSONObject("udp"))) {
            row.put("peer_identity", "private-identity").put("transport_address", "udp:private-host")
        }
        val filtered = filterBackgroundFipsHealth(input)
        assertFalse(filtered.toString().contains("private"))
        assertEquals("process:2", filtered.getString("sample_id"))
        assertEquals("process:1", filtered.getJSONObject("interval").getString("since_sample_id"))
        input.getJSONObject("transports").getJSONObject("udp").put("rx_packets", 999)
        assertEquals(3, filtered.getJSONObject("transports").getJSONObject("udp").getInt("rx_packets"))
    }

    @Test fun unavailableQueriesNeverAcquireFabricatedCounters() {
        for (status in listOf("unavailable", "timeout", "query_error", "invalid_counters", "diagnostics_error")) {
            val input = JSONObject().put("valid", false).put("status", status)
                .put("scope", "connected_authenticated_peers").put("private_field", "secret")
            val result = filterBackgroundFipsHealth(input)
            assertEquals(3, result.length())
            assertFalse(result.getBoolean("valid"))
            assertEquals(status, result.getString("status"))
            input.put("connected_peer_count", 0)
            assertTrue(runCatching { filterBackgroundFipsHealth(input) }.isFailure)
        }
    }

    @Test fun rejectsMissingCoercedNegativeOverflowAndInconsistentCounts() {
        for (value in listOf(false, "3", 3.0, -1, JSONObject.NULL)) {
            val input = sample()
            input.getJSONObject("transports").getJSONObject("udp").put("rx_packets", value)
            assertTrue(runCatching { filterBackgroundFipsHealth(input) }.isFailure)
        }
        val missing = sample().apply { getJSONObject("transports").getJSONObject("udp").remove("rx_packets") }
        val overflow = JSONObject(sample().toString().replace("\"rx_packets\":3", "\"rx_packets\":9223372036854775808"))
        val mismatch = sample().put("unexpected_connected_peer_count", 1)
        val numericId = sample().put("sample_id", 7)
        for (input in listOf(missing, overflow, mismatch, numericId)) {
            assertTrue(runCatching { filterBackgroundFipsHealth(input) }.isFailure)
        }
        val maximum = sample().apply { getJSONObject("transports").getJSONObject("udp").put("rx_bytes", Long.MAX_VALUE) }
        assertEquals(Long.MAX_VALUE, filterBackgroundFipsHealth(maximum).getJSONObject("transports")
            .getJSONObject("udp").getLong("rx_bytes"))
    }

    @Test fun invalidIntervalsRequireNullDeltasAndBaselineHasNoPreviousSample() {
        for (reason in listOf("baseline", "endpoint_changed", "peer_changed", "counter_reset", "no_elapsed_time")) {
            val input = sample()
            val interval = input.getJSONObject("interval").put("valid", false).put("reason", reason)
                .put("transport_deltas", JSONObject.NULL)
            if (reason == "baseline") interval.put("since_sample_id", JSONObject.NULL).put("elapsed_ms", JSONObject.NULL)
            if (reason == "no_elapsed_time") interval.put("elapsed_ms", JSONObject.NULL)
            assertFalse(filterBackgroundFipsHealth(input).getJSONObject("interval").getBoolean("valid"))
            interval.put("transport_deltas", JSONObject())
            assertTrue(runCatching { filterBackgroundFipsHealth(input) }.isFailure)
        }
        val missing = sample().apply { getJSONObject("interval").remove("elapsed_ms") }
        val sameId = sample().apply { getJSONObject("interval").put("since_sample_id", "process:2") }
        val zeroTime = sample().apply { getJSONObject("interval").put("elapsed_ms", 0) }
        for (input in listOf(missing, sameId, zeroTime)) {
            assertTrue(runCatching { filterBackgroundFipsHealth(input) }.isFailure)
        }
    }
}
