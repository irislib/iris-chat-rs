package to.iris.chat.push

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.debug.filterBackgroundFipsServices

class BackgroundFipsServicesSnapshotTest {
    private fun sample(): JSONObject {
        val services = JSONObject()
        for ((name, port) in mapOf("pubsub" to 7368, "hashtree" to 39018)) {
            val rows = JSONArray()
            for (transport in listOf("udp", "ethernet", "tcp", "tor", "websocket", "webrtc", "ble", "sim", "other")) {
                rows.put(JSONObject().put("transport", transport).put("submitted_packets", 0)
                    .put("fips_payload_bytes", 0).put("ethernet_framing_bytes", 0))
            }
            services.put(name, JSONObject().put("service_port", port).put("ambiguous_port_datagrams", 0)
                .put("discarded_outputs", 0).put("transports", rows))
        }
        return JSONObject().put("valid", true).put("status", "available")
            .put("scope", "locally_originated_service_carrier_submissions")
            .put("sample_id", "sample:1").put("epoch_id", "epoch:1").put("elapsed_ms", 0)
            .put("services", services).put("pubsub_delivery", JSONObject("""{
                "req_frames_received":0,"close_frames_received":0,"event_frames_received":0,
                "inv_frames_received":0,"want_frames_received":0,"want_frames_sent":0,
                "subscription_events_received":0,"expired_wants":0,"provider_cooldowns":0,
                "tcp_receive_batches":0,"tcp_datagrams_received":0,"tcp_datagrams_rejected":0,
                "tcp_poll_turns":0,"transport_errors":0
            }"""))
    }

    private fun service(value: JSONObject) = value.getJSONObject("services").getJSONObject("pubsub")
    private fun row(value: JSONObject) = service(value).getJSONArray("transports").getJSONObject(0)

    @Test fun reconstructsEveryNestedObjectWithoutPrivateFields() {
        val input = sample()
        for (part in listOf(input, service(input), row(input), input.getJSONObject("pubsub_delivery"))) {
            part.put("peer_identity", "private-id").put("address", "private-address")
        }
        val result = filterBackgroundFipsServices(input)
        assertFalse(result.toString().contains("private"))
        assertEquals(14, result.getJSONObject("pubsub_delivery").length())
        assertEquals(2, result.getJSONObject("services").length())
        assertEquals(9, service(result).getJSONArray("transports").length())
        assertEquals("epoch:1", result.getString("epoch_id"))
        row(input).put("submitted_packets", 99)
        assertEquals(0, row(result).getLong("submitted_packets"))
    }

    @Test fun queryFailuresRemainExplicitWithoutCounters() {
        for (status in listOf("unavailable", "query_error", "timeout", "invalid_counters")) {
            val input = JSONObject().put("valid", false).put("status", status)
                .put("scope", "locally_originated_service_carrier_submissions").put("private", "secret")
            val result = filterBackgroundFipsServices(input)
            assertFalse(result.getBoolean("valid"))
            assertEquals(3, result.length())
            for (field in listOf("services", "elapsed_ms", "pubsub_delivery", "sample_id", "epoch_id")) {
                input.put(field, JSONObject.NULL)
                assertTrue(runCatching { filterBackgroundFipsServices(input) }.isFailure)
                input.remove(field)
            }
        }
    }

    @Test fun rejectsCoercedMissingNegativeAndOverflowCounts() {
        for (bad in listOf(false, "1", 1.0, -1, JSONObject.NULL)) {
            val input = sample(); row(input).put("fips_payload_bytes", bad)
            assertTrue(runCatching { filterBackgroundFipsServices(input) }.isFailure)
        }
        val input = sample(); row(input).put("fips_payload_bytes", Long.MAX_VALUE)
        assertEquals(Long.MAX_VALUE, row(filterBackgroundFipsServices(input)).getLong("fips_payload_bytes"))
        val overflow = JSONObject(input.toString().replace(Long.MAX_VALUE.toString(), "9223372036854775808"))
        assertTrue(runCatching { filterBackgroundFipsServices(overflow) }.isFailure)
        val missing = sample(); missing.getJSONObject("pubsub_delivery").remove("transport_errors")
        assertTrue(runCatching { filterBackgroundFipsServices(missing) }.isFailure)
    }

    @Test fun requiresExactServicePortsTransportRowsAndObservationIds() {
        for (mutate in listOf<(JSONObject) -> Unit>(
            { it.getJSONObject("services").remove("hashtree") },
            { it.getJSONObject("services").put("calls", JSONObject()) },
            { service(it).put("service_port", 39018) },
            { service(it).getJSONArray("transports").remove(8) },
            { row(it).put("transport", "ethernet") },
            { it.put("sample_id", "") }, { it.put("epoch_id", 1) }, { it.put("elapsed_ms", -1) },
            { it.put("valid", 1) }, { it.put("scope", "all_traffic") }, { it.put("status", "timeout") }
        )) {
            val input = sample(); mutate(input)
            assertTrue(runCatching { filterBackgroundFipsServices(input) }.isFailure)
        }
    }

    @Test fun absentDeliveryIsDifferentFromExplicitNull() {
        val input = sample().put("pubsub_delivery", JSONObject.NULL)
        assertTrue(filterBackgroundFipsServices(input).isNull("pubsub_delivery"))
        input.remove("pubsub_delivery")
        assertTrue(runCatching { filterBackgroundFipsServices(input) }.isFailure)
    }
}
