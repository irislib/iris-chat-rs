package social.innode.ndr.demo.push

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import social.innode.ndr.demo.IrisChatApp

class IrisFirebaseMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        Log.d(TAG, "FCM token refreshed")
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val payloadJson = message.toPayloadJson()
        val appManager = (applicationContext as? IrisChatApp)?.container?.appManager
        // Block here on purpose. Firebase keeps the wakelock alive for as
        // long as onMessageReceived is on-stack, so a quick suspend block
        // is the right shape for "load secrets + decrypt + post" in
        // background or killed-app states. Decrypt + filesystem reads are
        // a few ms; any longer and we fall through anyway.
        val resolution =
            runCatching {
                if (appManager != null) {
                    runBlocking { appManager.decryptOrResolveNotificationPayload(payloadJson) }
                } else {
                    social.innode.ndr.demo.rust
                        .resolveMobilePushNotificationPayload(payloadJson)
                }
            }.getOrElse { error ->
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
