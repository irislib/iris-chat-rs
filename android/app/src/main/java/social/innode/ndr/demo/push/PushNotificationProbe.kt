package social.innode.ndr.demo.push

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import org.json.JSONObject
import social.innode.ndr.demo.rust.MobilePushNotificationResolution

object PushNotificationProbe {
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
