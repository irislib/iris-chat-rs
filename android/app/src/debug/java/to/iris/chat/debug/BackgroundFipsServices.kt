package to.iris.chat.debug

import org.json.JSONArray
import org.json.JSONObject

private const val serviceScope = "locally_originated_service_carrier_submissions"
private val servicePorts = mapOf("pubsub" to 7368L, "hashtree" to 39018L)
private val carrierNames = listOf("udp", "ethernet", "tcp", "tor", "websocket", "webrtc", "ble", "sim", "other")
private val serviceCounts = listOf("ambiguous_port_datagrams", "discarded_outputs")
private val carrierCounts = listOf("submitted_packets", "fips_payload_bytes", "ethernet_framing_bytes")
private val deliveryCounts = listOf("req_frames_received", "close_frames_received", "event_frames_received",
    "inv_frames_received", "want_frames_received", "want_frames_sent", "subscription_events_received",
    "expired_wants", "provider_cooldowns", "tcp_receive_batches", "tcp_datagrams_received",
    "tcp_datagrams_rejected", "tcp_poll_turns", "transport_errors")

private fun JSONObject.serviceCount(key: String): Long {
    val value = get(key)
    require(value is Int || value is Long)
    return (value as Number).toLong().also { require(it >= 0) }
}

private fun JSONObject.serviceId(key: String): String =
    (get(key) as? String ?: error("Invalid service observation identity")).also { require(it.isNotEmpty()) }

private fun serviceCounters(source: JSONObject, fields: List<String>): JSONObject =
    JSONObject().also { result -> fields.forEach { result.put(it, source.serviceCount(it)) } }

/** Allowlisted local submissions, never wire totals, peer identities, or application delivery proof. */
internal fun filterBackgroundFipsServices(source: JSONObject): JSONObject {
    val valid = source.get("valid") as? Boolean ?: error("Invalid service validity")
    val status = source.get("status") as? String ?: error("Invalid service status")
    require(source.get("scope") == serviceScope)
    val result = JSONObject().put("valid", valid).put("status", status).put("scope", serviceScope)
    if (!valid) {
        require(status in setOf("unavailable", "query_error", "timeout", "invalid_counters"))
        listOf("sample_id", "epoch_id", "elapsed_ms", "services", "pubsub_delivery").forEach { require(!source.has(it)) }
        return result
    }
    require(status == "available")
    result.put("sample_id", source.serviceId("sample_id")).put("epoch_id", source.serviceId("epoch_id"))
        .put("elapsed_ms", source.serviceCount("elapsed_ms"))
    val services = source.getJSONObject("services")
    require(services.keys().asSequence().toSet() == servicePorts.keys)
    val filtered = JSONObject()
    servicePorts.forEach { (name, port) ->
        val service = services.getJSONObject(name)
        require(service.serviceCount("service_port") == port)
        val rows = service.getJSONArray("transports")
        require(rows.length() == carrierNames.size)
        val transports = JSONArray()
        carrierNames.forEachIndexed { index, transport ->
            val row = rows.getJSONObject(index)
            require(row.get("transport") == transport)
            transports.put(serviceCounters(row, carrierCounts).put("transport", transport))
        }
        filtered.put(name, serviceCounters(service, serviceCounts).put("service_port", port).put("transports", transports))
    }
    result.put("services", filtered)
    val delivery = source.get("pubsub_delivery") // Explicit null is allowed; missing is an error.
    return result.put("pubsub_delivery", if (delivery === JSONObject.NULL) JSONObject.NULL
        else serviceCounters(delivery as? JSONObject ?: error("Invalid pubsub delivery"), deliveryCounts))
}
