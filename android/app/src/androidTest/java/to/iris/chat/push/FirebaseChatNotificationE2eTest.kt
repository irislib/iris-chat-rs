package to.iris.chat.push

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.os.Bundle
import android.os.SystemClock
import android.util.Base64
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
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.IrisChatApp

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
        requireHarnessInvocation("FCM token report requires targeted Firebase E2E invocation")
        val token = fcmToken()
        assertTrue("FCM token was empty", token.isNotBlank())
        reportStatus(
            "fcm_token" to token,
            "app_package" to context.packageName,
        )
    }

    @Test
    fun report_last_push_resolution() {
        val snapshot = PushNotificationProbe.snapshot(context)
        val rawPayload = snapshot.optString("raw_payload_json")
        if (rawPayload.isBlank() && arguments.getString("class").isNullOrBlank()) {
            assumeTrue("Push resolution report requires a recorded push payload", false)
        }
        assertTrue("No recorded push payload", rawPayload.isNotBlank())
        val appManager = (context as IrisChatApp).container.appManager
        val resolution =
            kotlinx.coroutines.runBlocking {
                appManager.decryptOrResolveNotificationPayload(rawPayload)
            }
        reportStatus(
            "should_show" to resolution.shouldShow.toString(),
            "title" to resolution.title,
            "body" to resolution.body,
            "payload" to resolution.payloadJson,
        )
    }

    @Test
    fun wait_for_firebase_chat_notification() {
        val expectedBody = requiredArg("message")
        val expectedTitle = optionalArg("expected_title")
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 120_000L
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
                if (expectedTitle != null && active.optString("title") != expectedTitle) {
                    return@waitForSnapshot null
                }
                candidate
            }

        assertEquals(expectedBody, snapshot.optString("body"))
        expectedTitle?.let { assertEquals(it, snapshot.optString("title")) }
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
    fun wait_for_active_chat_push_suppression() {
        val expectedBody = requiredArg("message")
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 120_000L
        val snapshot =
            waitForSnapshot("active-chat push suppression", timeoutMs) {
                val candidate = PushNotificationProbe.snapshot(context)
                if (candidate.optString("error").isNotEmpty()) {
                    throw AssertionError("Push notification resolution failed: ${candidate.optString("error")}")
                }
                if (candidate.optString("body") != expectedBody) {
                    return@waitForSnapshot null
                }
                if (candidate.optString("blocked_reason") != "active_chat_open") {
                    return@waitForSnapshot null
                }
                if (activeNotificationSnapshots().any { it.body == expectedBody }) {
                    throw AssertionError("Expected no active Android notification for open chat body `$expectedBody`")
                }
                candidate
            }

        reportStatus(
            "suppressed" to "true",
            "blocked_reason" to snapshot.optString("blocked_reason"),
            "title" to snapshot.optString("title"),
            "body" to snapshot.optString("body"),
            "snapshot" to snapshot.toString(),
        )
    }

    @Test
    fun wait_for_firebase_chat_notifications() {
        val expectedBodies = jsonStringArray(requiredArg("messages_json"))
        val expectedTitle = optionalArg("expected_title")
        assertTrue("Expected at least one notification body", expectedBodies.isNotEmpty())
        val timeoutMs = optionalArg("timeout_ms")?.toLongOrNull() ?: 120_000L
        val snapshot =
            waitForSnapshot("Firebase chat notifications", timeoutMs) {
                val candidate = PushNotificationProbe.snapshot(context)
                if (candidate.optString("error").isNotEmpty()) {
                    throw AssertionError("Push notification resolution failed: ${candidate.optString("error")}")
                }
                if (candidate.optString("blocked_reason").isNotEmpty()) {
                    throw AssertionError("Push notification was blocked: ${candidate.optString("blocked_reason")}")
                }
                val active = activeNotificationSnapshots()
                val activeBodies = active.map { it.body }
                val matchedBodies =
                    expectedBodies.all { body ->
                        active.any { notification ->
                            notification.body == body &&
                                (expectedTitle == null || notification.title == expectedTitle)
                        }
                    }
                if (matchedBodies) {
                    JSONObject()
                        .put("probe", candidate)
                        .put("active_bodies", JSONArray(activeBodies))
                        .put(
                            "active_titles",
                            JSONArray(active.filter { expectedBodies.contains(it.body) }.map { it.title }),
                        )
                } else {
                    null
                }
            }

        reportStatus(
            "received" to "true",
            "expected_title" to expectedTitle.orEmpty(),
            "expected_bodies" to JSONArray(expectedBodies).toString(),
            "active_bodies" to snapshot.optJSONArray("active_bodies")?.toString().orEmpty(),
            "active_titles" to snapshot.optJSONArray("active_titles")?.toString().orEmpty(),
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

    private fun activeNotificationSnapshots(): List<NotificationSnapshot> {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        return manager.activeNotifications.mapNotNull { statusBarNotification ->
            val extras = statusBarNotification.notification.extras
            val body = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() ?: return@mapNotNull null
            NotificationSnapshot(
                title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty(),
                body = body,
            )
        }
    }

    private fun jsonStringArray(raw: String): List<String> {
        val array = JSONArray(raw)
        return (0 until array.length()).mapNotNull { index ->
            array.optString(index).takeIf { it.isNotBlank() }
        }
    }

    private fun requiredArg(name: String): String =
        optionalArg(name)
            ?: run {
                if (arguments.getString("class").isNullOrBlank()) {
                    assumeTrue("Push harness action requires instrumentation argument: $name", false)
                }
                throw AssertionError("Missing instrumentation argument `$name`")
            }

    private fun optionalArg(name: String): String? =
        arguments.getString("${name}_b64")
            ?.let { String(Base64.decode(it, Base64.NO_WRAP or Base64.URL_SAFE), Charsets.UTF_8) }
            ?.takeIf { it.isNotBlank() }
            ?: arguments.getString(name)?.takeIf { it.isNotBlank() }

    private fun requireHarnessInvocation(reason: String) {
        if (arguments.getString("class").isNullOrBlank()) {
            assumeTrue(reason, false)
        }
    }

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

    private data class NotificationSnapshot(
        val title: String,
        val body: String,
    )
}
