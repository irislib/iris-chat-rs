package to.iris.chat.push

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.content.ContextCompat
import to.iris.chat.MainActivity
import to.iris.chat.R
import to.iris.chat.rust.MobilePushNotificationResolution

object MobilePushNotifier {
    fun show(
        context: Context,
        resolution: MobilePushNotificationResolution,
    ) {
        if (!notificationsAllowed(context)) {
            PushNotificationProbe.recordNotificationBlocked(context, "permission_denied")
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        ensureChannel(manager)

        val title = resolution.title.ifBlank { "Iris Chat" }
        val body = resolution.body.ifBlank { "New message" }
        val intent =
            Intent(context, MainActivity::class.java)
                .setAction("to.iris.chat.OPEN_CHAT_LIST")
                .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
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
                .build()
        val notificationId = resolution.payloadJson.hashCode() and Int.MAX_VALUE
        runCatching { manager.notify(notificationId, notification) }
            .onSuccess { PushNotificationProbe.recordNotificationShown(context, notificationId) }
            .onFailure { error ->
                Log.w(TAG, "Failed to show push notification", error)
                PushNotificationProbe.recordNotificationBlocked(context, error.javaClass.simpleName)
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

    private const val TAG = "IrisPush"
    const val CHANNEL_ID = "iris_chat_messages"
    private val VIBRATION_PATTERN = longArrayOf(0, 220, 90, 220)
}
