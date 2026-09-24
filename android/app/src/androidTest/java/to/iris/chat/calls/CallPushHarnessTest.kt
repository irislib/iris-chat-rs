package to.iris.chat.calls

import android.Manifest
import android.content.Context
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.MainActivity
import to.iris.chat.RealRelayHarnessBase
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.buildMobilePushListSubscriptionsRequest

/** Opt-in setup for a real FCM -> stopped app -> system incoming call test. */
@RunWith(AndroidJUnit4::class)
class CallPushHarnessTest : RealRelayHarnessBase() {
    @get:Rule override val activityRule = ActivityScenarioRule(MainActivity::class.java)

    @Test fun prepare_incoming_call_push() {
        assumeTrue("Needs a fresh call fixture", optionalArg("call_invite") != null)
        val invite = requiredArg("call_invite")
        val owner = requiredArg("call_owner")
        val device = requiredArg("call_device")
        val relay = requiredArg("call_relay")
        val server = requiredArg("call_push_server")
        listOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.CAMERA, Manifest.permission.POST_NOTIFICATIONS).forEach {
            instrumentation.uiAutomation.grantRuntimePermission(appPackageName(), it)
        }
        val manager = appManager()
        val account = ensureLoggedIn(createIfMissing = true)
        waitForPersistedDeviceSecret()
        val saved = instrumentation.targetContext.getSharedPreferences("call_push_harness", Context.MODE_PRIVATE)
        if (!saved.contains("preferences")) {
            val prefs = manager.preferences.value
            saved.edit().putString("preferences", JSONObject().apply {
                put("voice", prefs.voiceCallsEnabled); put("video", prefs.videoCallsEnabled)
                put("notifications", prefs.desktopNotificationsEnabled)
                put("server", prefs.mobilePushServerUrl)
                put("relays", org.json.JSONArray(prefs.nostrRelayUrls))
            }.toString()).commit()
        }
        manager.dispatch(AppAction.SetNostrRelays(listOf(relay)))
        manager.dispatch(AppAction.SetMobilePushServerUrl(server))
        manager.dispatch(AppAction.SetDesktopNotificationsEnabled(false))
        manager.dispatch(AppAction.SetVoiceCallsEnabled(true))
        manager.dispatch(AppAction.SetVideoCallsEnabled(true))
        manager.dispatch(AppAction.AcceptInvite(invite))
        waitForState("accepted call fixture", timeoutMs = 60_000) {
            manager.state.value.chatList.firstOrNull { it.chatId == owner }
        }
        manager.dispatch(AppAction.SetMessageRequestAccepted(owner))
        manager.sendText(owner, "Call wakeup test setup")
        waitForState("verified caller device", timeoutMs = 60_000) {
            manager.state.value.mobilePush.takeIf { device in it.callAuthorPubkeys }
        }
        val secret = runBlocking { manager.exportOwnerNsec() } ?: error("Test account secret unavailable")
        waitForState("call push registered with the existing service", timeoutMs = 60_000) {
            val request = buildMobilePushListSubscriptionsRequest(secret, "android", false, server)!!
            val connection = URL(request.url).openConnection() as HttpURLConnection
            try {
                connection.connectTimeout = 3_000; connection.readTimeout = 3_000
                connection.setRequestProperty("authorization", request.authorizationHeader)
                val subscriptions = JSONObject(connection.inputStream.bufferedReader().use { it.readText() })
                subscriptions.keys().asSequence().any { id ->
                    val sub = subscriptions.getJSONObject(id)
                    val filter = sub.optJSONObject("filter")
                    val authors = filter?.optJSONArray("authors")
                    (sub.optJSONArray("fcm_tokens")?.length() ?: 0) > 0 &&
                        filter?.optJSONArray("kinds")?.optInt(0) == 21111 &&
                        authors != null && (0 until authors.length()).any { authors.optString(it) == device }
                }.takeIf { it }
            } finally { connection.disconnect() }
        }
        reportStatus("call_push_ready" to "true", "owner" to account.publicKeyHex,
            "device" to account.devicePublicKeyHex)
    }

    @Test fun restore_after_call_push() {
        val saved = instrumentation.targetContext.getSharedPreferences("call_push_harness", Context.MODE_PRIVATE)
        val original = saved.getString("preferences", null)?.let(::JSONObject) ?: return
        ensureLoggedIn()
        appManager().dispatch(AppAction.SetVoiceCallsEnabled(false))
        appManager().dispatch(AppAction.SetVideoCallsEnabled(false))
        waitForState("calls disabled") { true.takeIf { appManager().state.value.mobilePush.callAuthorPubkeys.isEmpty() } }
        // Allow the existing subscription worker to unregister before the harness exits.
        Thread.sleep(3_000)
        val manager = appManager()
        val relays = original.getJSONArray("relays")
        manager.dispatch(AppAction.SetNostrRelays((0 until relays.length()).map { relays.getString(it) }))
        manager.dispatch(AppAction.SetMobilePushServerUrl(original.getString("server")))
        manager.dispatch(AppAction.SetDesktopNotificationsEnabled(original.getBoolean("notifications")))
        manager.dispatch(AppAction.SetVoiceCallsEnabled(original.getBoolean("voice")))
        manager.dispatch(AppAction.SetVideoCallsEnabled(original.getBoolean("video")))
        waitForState("test preferences restored") {
            manager.preferences.value.takeIf { it.voiceCallsEnabled == original.getBoolean("voice") &&
                it.videoCallsEnabled == original.getBoolean("video") && it.nostrRelayUrls.size == relays.length() }
        }
        saved.edit().clear().commit()
    }
}
