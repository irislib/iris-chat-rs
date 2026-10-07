package to.iris.chat.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import org.json.JSONObject
import to.iris.chat.core.pushNotificationChatCandidates
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.core.content.ContextCompat
import to.iris.chat.MainActivity
import to.iris.chat.R
import to.iris.chat.rust.MobilePushNotificationResolution
import to.iris.chat.rust.NotificationCandidate
import to.iris.chat.rust.ChatThreadSnapshot

object MobilePushNotifier {
    fun show(
        context: Context,
        resolution: MobilePushNotificationResolution,
        owner: String?,
        tag: String? = null,
        id: Int = resolution.payloadJson.hashCode() and Int.MAX_VALUE,
    ) {
        if (!notificationsAllowed(context)) {
            PushNotificationProbe.recordNotificationBlocked(context, "permission_denied")
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        ensureChannel(manager)

        val title = resolution.title.ifBlank { "Iris Chat" }
        val body = resolution.body.ifBlank { "New message" }
        val intent = launchIntent(context, resolution.payloadJson, owner)
        val pendingIntent =
            PendingIntent.getActivity(
                context,
                0,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            Notification.Builder(context, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(Notification.BigTextStyle().bigText(body))
                .setContentIntent(pendingIntent)
                .setAutoCancel(true)
                .setShowWhen(true)
                .addExtras(Bundle().apply { putString(PAYLOAD_KEY, resolution.payloadJson) })
                .build()
        runCatching { manager.notify(tag, id, notification) }
            .onSuccess { PushNotificationProbe.recordNotificationShown(context, id) }
            .onFailure { error ->
                Log.w(TAG, "Failed to show push notification", error)
                PushNotificationProbe.recordNotificationBlocked(context, error.javaClass.simpleName)
            }
    }

    fun showLocal(context: Context, candidate: NotificationCandidate, owner: String) {
        val payload = JSONObject().put("chat_id", candidate.chatId).toString()
        show(context, MobilePushNotificationResolution(true, candidate.title, candidate.body, payload),
            owner, localTag(owner, candidate.chatId), 0)
    }

    fun dismissLocalRead(context: Context, chats: List<ChatThreadSnapshot>, owner: String) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        val unreadTags = chats.filter { it.unreadCount > 0uL }.map { localTag(owner, it.chatId) }.toSet()
        manager.activeNotifications.filter {
            it.id == 0 && it.tag?.startsWith("$LOCAL_TAG_PREFIX$owner:") == true && it.tag !in unreadTags
        }
            .forEach { manager.cancel(it.tag, it.id) }
    }

    private const val LOCAL_TAG_PREFIX = "local-message:"
    private fun localTag(owner: String, chat: String) = "$LOCAL_TAG_PREFIX$owner:$chat"

    internal fun launchIntent(context: Context, payload: String, owner: String?): Intent {
        val chatId = runCatching { pushNotificationChatCandidates(JSONObject(payload)).firstOrNull() }.getOrNull()
        return Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .apply {
                if (chatId == null || owner.isNullOrBlank()) {
                    action = "to.iris.chat.OPEN_CHAT_LIST"
                } else {
                    action = ACTION_OPEN_CHAT
                    // PendingIntent identity excludes extras. Give each chat its
                    // own URI so a newer banner cannot redirect an older one.
                    data = Uri.Builder().scheme("irischat").authority("notification").appendPath(owner).appendPath(chatId).build()
                    putExtra(CHAT_ID_EXTRA, chatId)
                    putExtra(OWNER_EXTRA, owner)
                }
            }
    }

    const val ACTION_OPEN_CHAT = "to.iris.chat.OPEN_NOTIFICATION_CHAT"
    const val CHAT_ID_EXTRA = "notificationChatId"
    const val OWNER_EXTRA = "notificationOwner"

    fun dismissAll(context: Context) {
        runCatching { context.getSystemService(NotificationManager::class.java)?.cancelAll() }
            .onFailure { Log.w(TAG, "Failed to clear notifications", it) }
    }

    fun dismissRead(context: Context, dataDir: String, owner: String, device: String) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        val candidates = manager.activeNotifications.mapNotNull { notification ->
            // Live notifications are cleared from the authoritative chat snapshot.
            // They contain no encrypted push event and need no database/decrypt pass.
            if (notification.tag?.startsWith(LOCAL_TAG_PREFIX) == true) return@mapNotNull null
            notification.notification.extras.getString(PAYLOAD_KEY)?.let { payload ->
                notification to payload
            }
        }
        if (candidates.isEmpty()) return
        val indexes = to.iris.chat.rust.readMobilePushNotificationIndexes(
            dataDir, owner, device, candidates.map { it.second },
        )
        indexes.forEach { index ->
            candidates.getOrNull(index.toInt())?.first?.let { notification ->
                manager.cancel(notification.tag, notification.id)
            }
        }
    }

    private fun ensureChannel(manager: NotificationManager) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return
        }
        val existing = manager.getNotificationChannel(CHANNEL_ID)
        if (existing != null) {
            return
        }
        val channel =
            NotificationChannel(
                CHANNEL_ID,
                "Messages",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                enableVibration(true)
                vibrationPattern = VIBRATION_PATTERN
                setSound(DEFAULT_SOUND_URI, ALERT_AUDIO_ATTRIBUTES)
                setShowBadge(true)
            }
        manager.createNotificationChannel(channel)
    }

    private fun notificationsAllowed(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return true
        }
        return ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
    }

    private const val PAYLOAD_KEY = "iris_push_payload"
    private const val TAG = "IrisPush"
    const val CHANNEL_ID = "iris_chat_message_alerts"
    private val VIBRATION_PATTERN = longArrayOf(0, 220, 90, 220)
    private val DEFAULT_SOUND_URI = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION)
    private val ALERT_AUDIO_ATTRIBUTES =
        AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
}
