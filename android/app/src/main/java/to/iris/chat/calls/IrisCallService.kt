package to.iris.chat.calls

import android.app.NotificationChannel
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import to.iris.chat.IrisChatApp
import to.iris.chat.MainActivity
import to.iris.chat.R
import to.iris.chat.rust.AppAction

class IrisCallService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val container = (application as IrisChatApp).container
        val call = container.callRuntime.snapshot()
        if (call == null || call.phase == "ended") { dismiss(); return START_NOT_STICKY }
        if (intent?.action == "end" && intent.getStringExtra("callId") == call.callId) {
            container.appManager.dispatch(AppAction.EndCall(call.callId))
            dismiss()
            return START_NOT_STICKY
        }
        val incoming = call.phase == "incoming"
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("incoming-calls", "Incoming calls", NotificationManager.IMPORTANCE_HIGH).apply {
            setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE), AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
            enableVibration(true)
            lockscreenVisibility = Notification.VISIBILITY_PUBLIC
        })
        manager.createNotificationChannel(NotificationChannel("active-calls", "Calls in progress", NotificationManager.IMPORTANCE_LOW))
        // The identity includes the call ID: an old OS action must never answer
        // or reject a newer call whose PendingIntent reused the same request code.
        val uri = Uri.fromParts("iris-call", call.callId, null)
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java)
            .setAction("to.iris.chat.SHOW_CALL").setData(uri).putExtra("callId", call.callId)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val end = PendingIntent.getService(this, 1, Intent(this, IrisCallService::class.java)
            .setAction("end").setData(uri).putExtra("callId", call.callId), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val answer = PendingIntent.getActivity(this, 2, Intent(this, MainActivity::class.java)
            .setAction("to.iris.chat.ANSWER_CALL").setData(uri).putExtra("callId", call.callId)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val person = Person.Builder().setName(call.peerName.ifBlank { "Iris call" }).setImportant(true).build()
        val notification = NotificationCompat.Builder(this, if (incoming) "incoming-calls" else "active-calls")
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(call.peerName)
            .setContentText(if (incoming) "Incoming call" else if (call.phase == "connected") "Call in progress" else "Calling…")
            .setCategory(NotificationCompat.CATEGORY_CALL).setOngoing(true).setOnlyAlertOnce(true)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setContentIntent(open)
            .setFullScreenIntent(if (incoming) open else null, incoming)
            .setStyle(if (incoming) NotificationCompat.CallStyle.forIncomingCall(person, end, answer)
                .setIsVideo(call.videoCapable)
                else NotificationCompat.CallStyle.forOngoingCall(person, end))
            .build()
        if (incoming) notification.flags = notification.flags or Notification.FLAG_INSISTENT
        var types = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL else 0
        // Reserve while-in-use access while the outgoing call's activity is visible.
        // Capture still begins only after the other person answers.
        if (Build.VERSION.SDK_INT >= 30 && call.phase in listOf("outgoing", "ringing", "connected")) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (call.video) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        try {
            ServiceCompat.startForeground(this, 7401, notification, types)
            container.callRuntime.serviceStarted()
        } catch (_: RuntimeException) {
            container.appManager.dispatch(AppAction.EndCall(call.callId))
            dismiss()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        getSystemService(NotificationManager::class.java).cancel(7401)
        (application as IrisChatApp).container.callRuntime.serviceStopped()
        super.onDestroy()
    }

    private fun dismiss() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        getSystemService(NotificationManager::class.java).cancel(7401)
        stopSelf()
    }
}
