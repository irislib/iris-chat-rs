package social.innode.ndr.demo.push

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject
import social.innode.ndr.demo.rust.resolveMobilePushNotificationPayload

class IrisFirebaseMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        Log.d(TAG, "FCM token refreshed")
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val payloadJson = message.toPayloadJson()
        val resolution =
            runCatching { resolveMobilePushNotificationPayload(payloadJson) }
                .getOrElse { error ->
                    Log.w(TAG, "Failed to resolve FCM push payload", error)
                    PushNotificationProbe.recordError(this, payloadJson, error)
                    return
                }
        PushNotificationProbe.recordReceived(this, payloadJson, resolution)
        if (!resolution.shouldShow) {
            return
        }
        MobilePushNotifier.show(this, resolution)
    }

    private companion object {
        const val TAG = "IrisPush"
    }
}

private fun RemoteMessage.toPayloadJson(): String {
    val payload = JSONObject()
    data.toSortedMap().forEach { (key, value) ->
        payload.put(key, value)
    }
    notification?.title?.trim()?.takeIf { it.isNotEmpty() }?.let { title ->
        if (!payload.has("title")) {
            payload.put("title", title)
        }
    }
    notification?.body?.trim()?.takeIf { it.isNotEmpty() }?.let { body ->
        if (!payload.has("body")) {
            payload.put("body", body)
        }
    }
    return payload.toString()
}
