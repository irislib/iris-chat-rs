package to.iris.chat.debug

import org.json.JSONObject

private val transportNames = setOf("udp", "websocket", "webrtc", "tcp", "ble", "ethernet", "tor", "other")
private val trafficFields = listOf("rx_packets", "tx_packets", "rx_bytes", "tx_bytes")
private val peerFields = listOf("connected_peer_count", "configured_direct_peer_count",
    "connected_configured_direct_peer_count", "unexpected_connected_peer_count")
private val queryErrors = setOf("unavailable", "timeout", "query_error", "invalid_counters", "diagnostics_error")
private val invalidIntervals = setOf("baseline", "endpoint_changed", "peer_changed", "counter_reset", "no_elapsed_time")

private fun JSONObject.exactBoolean(key: String): Boolean = get(key) as? Boolean ?: error("Invalid boolean")

private fun JSONObject.exactCount(key: String): Long {
    val value = get(key)
    require(value is Int || value is Long)
    return (value as Number).toLong().also { require(it >= 0) }
}

private fun JSONObject.opaqueId(key: String): String =
    (get(key) as? String ?: error("Invalid sample ID")).also { require(it.isNotEmpty()) }

private fun transportRows(source: JSONObject, current: Boolean): JSONObject {
    val result = JSONObject()
    source.keys().forEach { name ->
        require(name in transportNames)
        val input = source.getJSONObject(name)
        val row = JSONObject()
        if (current) row.put("connected_peer_count", input.exactCount("connected_peer_count"))
        trafficFields.forEach { row.put(it, input.exactCount(it)) }
        result.put(name, row)
    }
    return result
}

/** Rebuild every nested object; never copy raw bundle fields into the cache. */
internal fun filterBackgroundFipsHealth(source: JSONObject): JSONObject {
    val valid = source.exactBoolean("valid")
    val status = source.get("status") as? String ?: error("Invalid status")
    require(source.get("scope") == "connected_authenticated_peers")
    val result = JSONObject().put("valid", valid).put("status", status)
        .put("scope", "connected_authenticated_peers")
    if (!valid) {
        require(status in queryErrors)
        (peerFields + listOf("sample_id", "transports", "interval")).forEach { require(!source.has(it)) }
        return result
    }
    require(status == "available")
    result.put("sample_id", source.opaqueId("sample_id"))
    peerFields.forEach { result.put(it, source.exactCount(it)) }
    val connected = result.getLong("connected_peer_count")
    val configuredConnected = result.getLong("connected_configured_direct_peer_count")
    require(configuredConnected <= result.getLong("configured_direct_peer_count"))
    require(Math.addExact(configuredConnected, result.getLong("unexpected_connected_peer_count")) == connected)
    val transports = transportRows(source.getJSONObject("transports"), current = true)
    var total = 0L
    transports.keys().forEach { total = Math.addExact(total, transports.getJSONObject(it).getLong("connected_peer_count")) }
    require(total == connected)
    result.put("transports", transports)

    val input = source.getJSONObject("interval")
    val intervalValid = input.exactBoolean("valid")
    val reason = input.get("reason") as? String ?: error("Invalid interval reason")
    // get() distinguishes explicit null from absent fields.
    val since = input.get("since_sample_id").let {
        if (it === JSONObject.NULL) JSONObject.NULL else input.opaqueId("since_sample_id")
    }
    val elapsed = input.get("elapsed_ms").let {
        if (it === JSONObject.NULL) JSONObject.NULL else input.exactCount("elapsed_ms").also { ms -> require(ms > 0) }
    }
    val deltas = input.get("transport_deltas")
    val interval = JSONObject().put("valid", intervalValid).put("reason", reason)
        .put("since_sample_id", since).put("elapsed_ms", elapsed)
    if (intervalValid) {
        require(reason == "comparable" && since !== JSONObject.NULL && elapsed !== JSONObject.NULL)
        require(since != result.getString("sample_id"))
        interval.put("transport_deltas", transportRows(deltas as? JSONObject ?: error("Missing deltas"), current = false))
    } else {
        require(reason in invalidIntervals && deltas === JSONObject.NULL)
        if (reason == "baseline") require(since === JSONObject.NULL && elapsed === JSONObject.NULL)
        interval.put("transport_deltas", JSONObject.NULL)
    }
    return result.put("interval", interval)
}
