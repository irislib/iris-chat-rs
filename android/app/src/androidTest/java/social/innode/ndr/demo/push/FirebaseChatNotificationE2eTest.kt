package social.innode.ndr.demo.push

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.os.Bundle
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.firebase.messaging.FirebaseMessaging
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class FirebaseChatNotificationE2eTest {
    private val instrumentation
        get() = InstrumentationRegistry.getInstrumentation()
    private val context
        get() = instrumentation.targetContext.applicationContext
    private val arguments
        get() = InstrumentationRegistry.getArguments()

    @Test
    fun clear_push_probe() {
        PushNotificationProbe.clear(context)
        reportStatus("cleared" to "true")
    }

    @Test
    fun report_fcm_token() {
        val token = fcmToken()
        assertTrue("FCM token was empty", token.isNotBlank())
        reportStatus(
            "fcm_token" to token,
            "app_package" to context.packageName,
        )
    }

    @Test
    fun wait_for_firebase_chat_notification() {
        val expectedBody = requiredArg("message")
        val timeoutMs = arguments.getString("timeout_ms")?.toLongOrNull() ?: 120_000L
        val snapshot =
            waitForSnapshot("Firebase chat notification", timeoutMs) {
                val candidate = PushNotificationProbe.snapshot(context)
                if (candidate.optString("body") != expectedBody) {
                    return@waitForSnapshot null
                }
                if (candidate.optString("error").isNotEmpty()) {
                    throw AssertionError("Push notification resolution failed: ${candidate.optString("error")}")
                }
                if (candidate.optString("blocked_reason").isNotEmpty()) {
                    throw AssertionError("Push notification was blocked: ${candidate.optString("blocked_reason")}")
                }
                if (candidate.optLong("shown_at_ms") <= 0L) {
                    return@waitForSnapshot null
                }
                val active = candidate.optJSONObject("active_notification")
                if (active?.optString("text") != expectedBody) {
                    return@waitForSnapshot null
                }
                candidate
            }

        assertEquals(expectedBody, snapshot.optString("body"))
        assertActiveNotificationBody(expectedBody)
        reportStatus(
            "received" to "true",
            "title" to snapshot.optString("title"),
            "body" to snapshot.optString("body"),
            "notification_id" to snapshot.optInt("notification_id").toString(),
            "snapshot" to snapshot.toString(),
        )
    }

    @Test
    fun wait_for_firebase_chat_notifications() {
        val expectedBodies = jsonStringArray(requiredArg("messages_json"))
        assertTrue("Expected at least one notification body", expectedBodies.isNotEmpty())
        val timeoutMs = arguments.getString("timeout_ms")?.toLongOrNull() ?: 120_000L
        val snapshot =
            waitForSnapshot("Firebase chat notifications", timeoutMs) {
                val candidate = PushNotificationProbe.snapshot(context)
                if (candidate.optString("error").isNotEmpty()) {
                    throw AssertionError("Push notification resolution failed: ${candidate.optString("error")}")
                }
                if (candidate.optString("blocked_reason").isNotEmpty()) {
                    throw AssertionError("Push notification was blocked: ${candidate.optString("blocked_reason")}")
                }
                val activeBodies = activeNotificationBodies()
                if (expectedBodies.all { body -> activeBodies.contains(body) }) {
                    JSONObject()
                        .put("probe", candidate)
                        .put("active_bodies", JSONArray(activeBodies))
                } else {
                    null
                }
            }

        reportStatus(
            "received" to "true",
            "expected_bodies" to JSONArray(expectedBodies).toString(),
            "active_bodies" to snapshot.optJSONArray("active_bodies").toString(),
            "snapshot" to snapshot.toString(),
        )
    }

    private fun fcmToken(): String {
        val deadline = SystemClock.elapsedRealtime() + 90_000L
        var lastError: Exception? = null
        while (SystemClock.elapsedRealtime() < deadline) {
            var token: String? = null
            var error: Exception? = null
            val latch = CountDownLatch(1)
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (task.isSuccessful) {
                    token = task.result
                } else {
                    error = task.exception
                }
                latch.countDown()
            }
            assertTrue("Timed out waiting for FCM token task", latch.await(30, TimeUnit.SECONDS))
            val normalized = token?.trim().orEmpty()
            if (normalized.isNotEmpty()) {
                return normalized
            }
            lastError = error
            SystemClock.sleep(2_000)
        }
        lastError?.let { throw it }
        return ""
    }

    private fun assertActiveNotificationBody(expectedBody: String) {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val active =
            manager.activeNotifications.firstOrNull {
                it.notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() == expectedBody
            }
        assertNotNull("Expected active Android notification body `$expectedBody`", active)
    }

    private fun activeNotificationBodies(): List<String> {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        return manager.activeNotifications.mapNotNull { statusBarNotification ->
            statusBarNotification.notification.extras
                .getCharSequence(Notification.EXTRA_TEXT)
                ?.toString()
        }
    }

    private fun jsonStringArray(raw: String): List<String> {
        val array = JSONArray(raw)
        return (0 until array.length()).mapNotNull { index ->
            array.optString(index).takeIf { it.isNotBlank() }
        }
    }

    private fun requiredArg(name: String): String =
        arguments.getString(name)?.takeIf { it.isNotBlank() }
            ?: throw AssertionError("Missing instrumentation argument `$name`")

    private fun waitForSnapshot(
        label: String,
        timeoutMs: Long,
        condition: () -> JSONObject?,
    ): JSONObject {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        var latest = PushNotificationProbe.snapshot(context)
        while (SystemClock.elapsedRealtime() < deadline) {
            condition()?.let { return it }
            latest = PushNotificationProbe.snapshot(context)
            SystemClock.sleep(500)
        }
        throw AssertionError("Timed out waiting for $label. Latest probe: $latest")
    }

    private fun reportStatus(vararg fields: Pair<String, String>) {
        val bundle = Bundle()
        fields.forEach { (key, value) -> bundle.putString(key, value) }
        instrumentation.sendStatus(0, bundle)
    }
}
