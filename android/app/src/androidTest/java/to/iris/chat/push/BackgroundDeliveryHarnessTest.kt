package to.iris.chat.push

import android.Manifest
import android.app.NotificationManager
import android.os.Build
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.MainActivity
import to.iris.chat.RealRelayHarnessBase
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.Screen

/** Only the separate backgroundtest package may create this disposable account. */
@RunWith(AndroidJUnit4::class)
class BackgroundDeliveryHarnessTest : RealRelayHarnessBase() {
    @get:Rule override val activityRule = ActivityScenarioRule(MainActivity::class.java)

    @Test fun prepare_background_receiver() {
        assumeTrue("Use the isolated background delivery harness", optionalArg("fixture_invite") != null)
        require(appPackageName() == "to.iris.chat.backgroundtest")
        listOf(Manifest.permission.POST_NOTIFICATIONS, Manifest.permission.RECORD_AUDIO,
            Manifest.permission.CAMERA).forEach {
            instrumentation.uiAutomation.grantRuntimePermission(appPackageName(), it)
        }
        val account = ensureLoggedIn(createIfMissing = true)
        waitForPersistedDeviceSecret()
        val manager = appManager()
        val owner = requiredArg("fixture_owner")
        val device = requiredArg("fixture_device")
        manager.dispatch(AppAction.SetNostrRelays(listOf(requiredArg("fixture_relay"))))
        val offlineLan = optionalArg("offline_lan") == "1"
        if (offlineLan && Build.VERSION.SDK_INT >= 33) {
            instrumentation.uiAutomation.grantRuntimePermission(appPackageName(), Manifest.permission.NEARBY_WIFI_DEVICES)
        }
        manager.dispatch(AppAction.SetNearbyEnabled(offlineLan))
        manager.dispatch(AppAction.SetNearbyLanEnabled(offlineLan))
        manager.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
        manager.dispatch(AppAction.SetDesktopNotificationsEnabled(true))
        manager.dispatch(AppAction.SetVoiceCallsEnabled(true))
        manager.dispatch(AppAction.SetVideoCallsEnabled(true))
        manager.dispatch(AppAction.AcceptInvite(requiredArg("fixture_invite")))
        waitForState("accepted fixture contact", timeoutMs = 60_000) {
            manager.state.value.chatList.firstOrNull { it.chatId == owner }
        }
        manager.dispatch(AppAction.SetMessageRequestAccepted(owner))
        manager.sendText(owner, "Background receiver ready")
        waitForState("verified fixture device", timeoutMs = 60_000) {
            true.takeIf { device in manager.state.value.mobilePush.callAuthorPubkeys }
        }
        waitForState("background receiver service") { true.takeIf { receiverActive() } }
        // Closing the activity leaves this chat in the router. Incoming messages
        // must still become unread and raise an alert after the screen turns off.
        manager.dispatch(AppAction.UpdateScreenStack(listOf(Screen.Chat(owner))))
        reportStatus("owner" to account.publicKeyHex, "device" to account.devicePublicKeyHex)
    }

    @Test fun stop_when_alerts_disabled() {
        assumeTrue("Use the isolated background delivery harness", optionalArg("background_harness") == "1")
        require(appPackageName() == "to.iris.chat.backgroundtest")
        ensureLoggedIn()
        val manager = appManager()
        manager.dispatch(AppAction.SetDesktopNotificationsEnabled(false))
        manager.dispatch(AppAction.SetVoiceCallsEnabled(false))
        manager.dispatch(AppAction.SetVideoCallsEnabled(false))
        waitForState("background receiver stopped") { true.takeIf { !receiverActive() } }
        assertTrue(!receiverActive())
    }

    private fun receiverActive() = instrumentation.targetContext
        .getSystemService(NotificationManager::class.java).activeNotifications
        .any { it.id == BackgroundMessageService.ID }
}
