package to.iris.chat.push

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject
import to.iris.chat.BuildConfig
import to.iris.chat.rust.CallSnapshot
import to.iris.chat.rust.MobilePushNotificationResolution

object PushNotificationProbe {
    /** Local, opt-in diagnostics only; never enabled by a push or production preference. */
    @Synchronized
    fun armCallProbe(context: Context) {
        if (!BuildConfig.DEBUG) return
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit()
            .putBoolean(KEY_CALL_PROBE_ENABLED, true)
            .putString(KEY_CALL_EVENTS, "[]")
            .commit()
    }

    @Synchronized
    fun disarmCallProbe(context: Context) {
        if (!BuildConfig.DEBUG) return
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit()
            .putBoolean(KEY_CALL_PROBE_ENABLED, false).commit()
    }

    @Synchronized
    fun recordCallEvent(
        context: Context,
        stage: String,
        callId: String? = null,
        runtimeCall: CallSnapshot? = null,
        coreCall: CallSnapshot? = null,
        pendingCallId: String? = null,
        previewCall: CallSnapshot? = null,
        rawPayloadJson: String? = null,
        detail: String? = null,
        notificationId: Int? = null,
    ) {
        if (!BuildConfig.DEBUG) return
        // Diagnostics must never alter call delivery if storage is unavailable.
        runCatching {
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            if (!prefs.getBoolean(KEY_CALL_PROBE_ENABLED, false)) return
            val state = JSONObject()
                .put("runtime_call", callState(runtimeCall))
                .put("core_call", callState(coreCall))
                .put("pending_call_id", pendingCallId ?: JSONObject.NULL)
            val event = JSONObject()
                .put("stage", stage)
                .put("call_id", callId ?: JSONObject.NULL)
                .put("state", state)
                .put("state_sha256", sha256(state.toString()))
                .put("preview_call", callState(previewCall))
                .put("detail", detail ?: JSONObject.NULL)
                .put("notification_id", notificationId ?: JSONObject.NULL)
            rawPayloadJson?.let { raw ->
                event.put("raw_payload_json", raw).put("raw_payload_sha256", sha256(raw))
                val outer = JSONObject(raw).opt("event")
                val parsed = if (outer is String) JSONObject(outer) else outer as? JSONObject
                event.put("outer_event_id", parsed?.optString("id") ?: JSONObject.NULL)
            }
            val previous = JSONArray(prefs.getString(KEY_CALL_EVENTS, "[]"))
            val fingerprint = sha256(event.toString())
            if (previous.optJSONObject(previous.length() - 1)?.optString("record_sha256") == fingerprint) return
            event.put("record_sha256", fingerprint).put("at_ms", System.currentTimeMillis())
            val events = JSONArray()
            for (index in maxOf(0, previous.length() - MAX_CALL_EVENTS + 1) until previous.length()) {
                events.put(previous.get(index))
            }
            events.put(event)
            // Persist before FCM releases its wake lease or a test kills the process.
            prefs.edit().putString(KEY_CALL_EVENTS, events.toString()).commit()
        }
    }

    private fun callState(call: CallSnapshot?): Any = call?.let {
        JSONObject().put("call_id", it.callId).put("phase", it.phase)
            .put("outgoing", it.outgoing).put("video", it.video)
            .put("media_connected", it.mediaConnected).put("end_reason", it.endReason ?: JSONObject.NULL)
    } ?: JSONObject.NULL

    private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
        .digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 0xff) }

    fun clear(context: Context) {
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit().clear().apply()
        context.getSystemService(NotificationManager::class.java)?.cancelAll()
    }

    fun recordReceived(
        context: Context,
        rawPayloadJson: String,
        resolution: MobilePushNotificationResolution,
    ) {
        context
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putLong(KEY_RECEIVED_AT_MS, System.currentTimeMillis())
            .putString(KEY_RAW_PAYLOAD_JSON, rawPayloadJson)
            .putBoolean(KEY_SHOULD_SHOW, resolution.shouldShow)
            .putString(KEY_TITLE, resolution.title)
            .putString(KEY_BODY, resolution.body)
            .putString(KEY_RESOLVED_PAYLOAD_JSON, resolution.payloadJson)
            .remove(KEY_ERROR)
            .apply()
    }

    fun recordError(
        context: Context,
        rawPayloadJson: String,
        error: Throwable,
    ) {
        context
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putLong(KEY_RECEIVED_AT_MS, System.currentTimeMillis())
            .putString(KEY_RAW_PAYLOAD_JSON, rawPayloadJson)
            .putString(KEY_ERROR, error.toString())
            .apply()
    }

    fun recordNotificationShown(
        context: Context,
        notificationId: Int,
    ) {
        context
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putLong(KEY_SHOWN_AT_MS, System.currentTimeMillis())
            .putInt(KEY_NOTIFICATION_ID, notificationId)
            .remove(KEY_BLOCKED_REASON)
            .apply()
    }

    fun recordNotificationBlocked(
        context: Context,
        reason: String,
    ) {
        context
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit()
            .putString(KEY_BLOCKED_REASON, reason)
            .apply()
    }

    fun snapshot(context: Context): JSONObject {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val activeNotification = activeNotificationSnapshot(context, prefs.getInt(KEY_NOTIFICATION_ID, -1))
        return JSONObject()
            .put("received_at_ms", prefs.getLong(KEY_RECEIVED_AT_MS, 0L))
            .put("shown_at_ms", prefs.getLong(KEY_SHOWN_AT_MS, 0L))
            .put("notification_id", prefs.getInt(KEY_NOTIFICATION_ID, -1))
            .put("raw_payload_json", prefs.getString(KEY_RAW_PAYLOAD_JSON, "") ?: "")
            .put("should_show", prefs.getBoolean(KEY_SHOULD_SHOW, false))
            .put("title", prefs.getString(KEY_TITLE, "") ?: "")
            .put("body", prefs.getString(KEY_BODY, "") ?: "")
            .put("resolved_payload_json", prefs.getString(KEY_RESOLVED_PAYLOAD_JSON, "") ?: "")
            .put("error", prefs.getString(KEY_ERROR, "") ?: "")
            .put("blocked_reason", prefs.getString(KEY_BLOCKED_REASON, "") ?: "")
            .put("active_notification", activeNotification)
            .put("call_probe_enabled", BuildConfig.DEBUG && prefs.getBoolean(KEY_CALL_PROBE_ENABLED, false))
            .put("call_events", JSONArray(prefs.getString(KEY_CALL_EVENTS, "[]")))
    }

    private fun activeNotificationSnapshot(
        context: Context,
        notificationId: Int,
    ): JSONObject {
        if (notificationId < 0) {
            return JSONObject()
        }
        val manager = context.getSystemService(NotificationManager::class.java) ?: return JSONObject()
        val statusBarNotification =
            manager.activeNotifications.firstOrNull { it.id == notificationId } ?: return JSONObject()
        val notification = statusBarNotification.notification
        return JSONObject()
            .put("id", statusBarNotification.id)
            .put("tag", statusBarNotification.tag ?: "")
            .put("title", notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: "")
            .put("text", notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() ?: "")
    }

    private const val PREFS_NAME = "iris_push_probe"
    private const val KEY_CALL_PROBE_ENABLED = "call_probe_enabled"
    private const val KEY_CALL_EVENTS = "call_events"
    private const val MAX_CALL_EVENTS = 128
    private const val KEY_RECEIVED_AT_MS = "received_at_ms"
    private const val KEY_SHOWN_AT_MS = "shown_at_ms"
    private const val KEY_NOTIFICATION_ID = "notification_id"
    private const val KEY_RAW_PAYLOAD_JSON = "raw_payload_json"
    private const val KEY_SHOULD_SHOW = "should_show"
    private const val KEY_TITLE = "title"
    private const val KEY_BODY = "body"
    private const val KEY_RESOLVED_PAYLOAD_JSON = "resolved_payload_json"
    private const val KEY_ERROR = "error"
    private const val KEY_BLOCKED_REASON = "blocked_reason"
}
