package to.iris.chat.push

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import to.iris.chat.IrisChatApp
import to.iris.chat.MainActivity
import to.iris.chat.R

/** No timer, polling loop, or CPU/Wi-Fi wake lock: the existing transport receives events. */
class BackgroundMessageService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Background receiving", NotificationManager.IMPORTANCE_LOW)
            .apply { setSound(null, null); setShowBadge(false) })
        val unrestricted = getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(packageName)
        val open = PendingIntent.getActivity(this, 7402,
            Intent(this, MainActivity::class.java).setAction(ACTION_ALLOW_BACKGROUND)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification).setContentTitle("Receiving messages and calls")
            .setContentText(if (unrestricted) null else "Tap to allow receiving with the screen off")
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true).setSilent(true)
            .setShowWhen(false).build()
        try {
            ServiceCompat.startForeground(this, ID, notification,
                if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0)
        } catch (error: RuntimeException) {
            android.util.Log.w("IrisPush", "Background receiving could not enter foreground", error)
            stopSelf()
            return START_NOT_STICKY
        }
        (application as IrisChatApp).container.backgroundDelivery.serviceStarted()
        return START_STICKY
    }

    override fun onDestroy() {
        (application as IrisChatApp).container.backgroundDelivery.serviceStopped()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        const val ACTION_ALLOW_BACKGROUND = "to.iris.chat.ALLOW_BACKGROUND_RECEIVING"
        const val CHANNEL = "background-receiving"
        const val ID = 7402
    }
}

class BackgroundMessageRestoreReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action !in setOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED)) return
        if (AndroidBackgroundDelivery.shouldRestore(context)) {
            ContextCompat.startForegroundService(context, Intent(context, BackgroundMessageService::class.java))
        }
    }
}
