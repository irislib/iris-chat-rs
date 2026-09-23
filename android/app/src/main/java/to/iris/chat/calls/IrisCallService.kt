package to.iris.chat.calls

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
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
        if (intent?.action == "end") container.appManager.dispatch(AppAction.EndCall(intent.getStringExtra("callId") ?: ""))
        val call = container.callRuntime.snapshot()
        if (call == null || call.phase == "ended") { stopSelf(); return START_NOT_STICKY }
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel("calls", "Calls", NotificationManager.IMPORTANCE_HIGH))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val end = PendingIntent.getService(this, 1, Intent(this, IrisCallService::class.java)
            .setAction("end").putExtra("callId", call.callId), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val answer = PendingIntent.getActivity(this, 2, Intent(this, MainActivity::class.java)
            .setAction("to.iris.chat.ANSWER_CALL").putExtra("callId", call.callId),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val person = Person.Builder().setName(call.peerName.ifBlank { "Iris call" }).setImportant(true).build()
        val incoming = call.phase == "incoming"
        val notification = NotificationCompat.Builder(this, "calls")
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(call.peerName)
            .setContentText(if (incoming) "Incoming call" else if (call.phase == "connected") "Call in progress" else "Calling…")
            .setCategory(NotificationCompat.CATEGORY_CALL).setOngoing(true).setOnlyAlertOnce(true)
            .setContentIntent(open)
            .setStyle(if (incoming) NotificationCompat.CallStyle.forIncomingCall(person, end, answer)
                else NotificationCompat.CallStyle.forOngoingCall(person, end))
            .build()
        var types = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL else 0
        // Reserve while-in-use access while the outgoing call's activity is visible.
        // Capture still begins only after the other person answers.
        if (Build.VERSION.SDK_INT >= 30 && call.phase in listOf("outgoing", "connected")) {
            types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (call.video) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        try {
            ServiceCompat.startForeground(this, 7401, notification, types)
            container.callRuntime.serviceStarted()
        } catch (_: RuntimeException) {
            container.appManager.dispatch(AppAction.EndCall(call.callId))
            stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        (application as IrisChatApp).container.callRuntime.serviceStopped()
        super.onDestroy()
    }
}
