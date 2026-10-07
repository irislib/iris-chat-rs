package to.iris.chat.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Process
import android.os.SystemClock
import android.util.AtomicFile
import java.io.File
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import to.iris.chat.BuildConfig
import to.iris.chat.IrisChatApp

/** One-shot, app-UID-only harness query. No timer or continuous debug snapshots. */
class BackgroundHealthReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (!BuildConfig.DEBUG || context.packageName != PACKAGE || intent.action != ACTION) return
        if (intent.getIntExtra("expected_pid", -1) != Process.myPid()) return
        val expectedRelay = intent.getStringExtra("expected_relay") ?: return
        val app = context.applicationContext as? IrisChatApp ?: return
        val pending = goAsync()
        Thread({
            try {
                val result = try {
                    // The raw bundle never leaves memory. Exporting does not foreground the app.
                    val bundle = runBlocking { app.container.appManager.exportSupportBundleJson() }
                    filterBackgroundHealth(JSONObject(bundle), expectedRelay,
                        requireFips = intent.getBooleanExtra("require_fips", false))
                } catch (_: Exception) {
                    JSONObject().put("valid", false).put("error", "invalid_health_snapshot")
                }
                result.put("schema_version", 1).put("pid", Process.myPid())
                    .put("elapsed_realtime_ms", SystemClock.elapsedRealtime())
                val file = AtomicFile(File(context.cacheDir, FILENAME))
                val stream = file.startWrite()
                try {
                    stream.write(result.toString().toByteArray(Charsets.UTF_8))
                    file.finishWrite(stream)
                } catch (error: Exception) {
                    file.failWrite(stream)
                    throw error
                }
            } catch (_: Exception) {
                // A failed cache write makes the host check time out; never crash the receiver app.
            } finally {
                pending.finish()
            }
        }, "background-health-once").start()
    }

    companion object {
        const val PACKAGE = "to.iris.chat.backgroundtest"
        const val ACTION = "to.iris.chat.BACKGROUND_HEALTH"
        const val FILENAME = "background-health.json"
    }
}

/** Allowlist aggregates; missing, coerced, or timed-out data must never look healthy. */
internal fun filterBackgroundHealth(bundle: JSONObject, expectedRelay: String, requireFips: Boolean = false): JSONObject {
    fun JSONObject.boolean(key: String): Boolean = get(key) as? Boolean ?: error("Invalid boolean")
    fun JSONObject.count(key: String): Long {
        val value = get(key)
        require(value is Int || value is Long)
        return (value as Number).toLong().also { require(it >= 0) }
    }
    require(!bundle.getJSONObject("ffi_queue").boolean("core_support_bundle_timed_out"))
    val relays = bundle.getJSONArray("relay_urls")
    val relayMatches = relays.length() == 1 && relays.getString(0) == expectedRelay
    val transport = bundle.getJSONObject("relay_transport")
    val phase = transport.getString("phase")
    require(phase in setOf("connecting", "publishing", "backoff", "connected", "offline"))
    val result = JSONObject().put("valid", true).put("expected_relay_matches", relayMatches)
        .put("configured_relay_count", relays.length()).put("phase", phase)
    listOf("connected_relay_count", "pending_relay_publish_count", "retry_backoff_attempt")
        .forEach { result.put(it, transport.count(it)) }
    listOf("connect_in_flight", "connect_dirty", "force_reconnect_dirty",
        "publish_drain_in_flight", "publish_drain_dirty")
        .forEach { result.put(it, transport.boolean(it)) }
    require(transport.has("next_retry_due_in_ms"))
    result.put("retry_scheduled", !transport.isNull("next_retry_due_in_ms"))
    if (!transport.isNull("next_retry_due_in_ms")) transport.count("next_retry_due_in_ms")
    if (bundle.has("fips_transport")) {
        result.put("fips_transport", filterBackgroundFipsHealth(bundle.getJSONObject("fips_transport")))
    } else {
        require(!requireFips)
    }
    return result
}
