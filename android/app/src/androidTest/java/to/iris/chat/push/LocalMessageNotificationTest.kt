package to.iris.chat.push

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.NotificationCandidate
import to.iris.chat.rust.buildLargeTestAppState

@RunWith(AndroidJUnit4::class)
class LocalMessageNotificationTest {
    @Test fun delivered_messages_clear_only_after_read_or_chat_removal() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        val manager = context.getSystemService(NotificationManager::class.java)
        val owner = "notification-test-${UUID.randomUUID()}"
        val chat = buildLargeTestAppState(1u, 1u, 0u).chatList.first().copy(unreadCount = 1u)
        val candidate = NotificationCandidate(chat.chatId, "Background sender", "New background message")
        val tag = "local-message:$owner:${chat.chatId}"
        try {
            MobilePushNotifier.showLocal(context, candidate, owner)
            awaitNotification { manager.activeNotifications.any { it.tag == tag } }
            val shown = manager.activeNotifications.single { it.tag == tag }
            assertEquals(candidate.body, shown.notification.extras.getString(Notification.EXTRA_TEXT))
            MobilePushNotifier.dismissLocalRead(context, listOf(chat), owner)
            assertTrue(manager.activeNotifications.any { it.tag == tag })
            MobilePushNotifier.dismissLocalRead(context, listOf(chat.copy(unreadCount = 0u)), owner)
            awaitNotification { manager.activeNotifications.none { it.tag == tag } }
            assertTrue(manager.activeNotifications.none { it.tag == tag })
            MobilePushNotifier.showLocal(context, candidate, owner)
            awaitNotification { manager.activeNotifications.any { it.tag == tag } }
            MobilePushNotifier.dismissLocalRead(context, emptyList(), owner)
            awaitNotification { manager.activeNotifications.none { it.tag == tag } }
            assertTrue(manager.activeNotifications.none { it.tag == tag })
        } finally {
            manager.cancel(tag, 0)
        }
    }

    private fun awaitNotification(ready: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        while (!ready() && SystemClock.elapsedRealtime() < deadline) Thread.sleep(20)
        assertTrue("Android notification did not reach the expected state", ready())
    }
}
