package to.iris.chat.push

import androidx.test.ext.junit.rules.ActivityScenarioRule
import java.net.DatagramSocket
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import to.iris.chat.MainActivity
import to.iris.chat.RealRelayHarnessBase
import to.iris.chat.debug.BackgroundControlProvider
import to.iris.chat.rust.AppAction

/** Fresh control app only. No permission grants and no access to other app accounts. */
class BackgroundControlHarnessTest : RealRelayHarnessBase() {
    @get:Rule override val activityRule = ActivityScenarioRule(MainActivity::class.java)

    private fun requireControl() {
        require(appPackageName() == BackgroundControlProvider.PACKAGE)
        require(optionalArg("background_control") == "1")
    }

    private fun disableDiscovery() {
        appManager().dispatch(AppAction.SetNearbyEnabled(false))
        appManager().dispatch(AppAction.SetNearbyLanEnabled(false))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(false))
    }

    @Test fun bootstrap_fresh_identity() {
        requireControl()
        require(appManager().state.value.account == null)
        disableDiscovery()
        val account = ensureLoggedIn(createIfMissing = true)
        waitForPersistedDeviceSecret()
        val debug = JSONObject(runBlocking { appManager().exportSupportBundleJson() })
        require(!debug.getJSONObject("ffi_queue").getBoolean("core_support_bundle_timed_out"))
        val fips = debug.getJSONObject("fips_transport")
        require(!fips.getBoolean("valid") && fips.getString("status") == "unavailable")
        val udpPort = DatagramSocket(0).use { it.localPort }
        require(udpPort in 1024..65535)
        reportStatus("owner" to account.publicKeyHex, "device" to account.devicePublicKeyHex,
            "device_npub" to account.deviceNpub, "udp_port" to udpPort.toString(), "mesh_absent" to "true")
    }

    @Test fun connect_control_peer() {
        requireControl()
        val account = ensureLoggedIn()
        require(account.publicKeyHex == requiredArg("fixture_account_owner"))
        disableDiscovery()
        val manager = appManager()
        manager.dispatch(AppAction.SetNostrRelays(listOf(requiredArg("fixture_relay"))))
        manager.dispatch(AppAction.SetDesktopNotificationsEnabled(true))
        manager.dispatch(AppAction.SetVoiceCallsEnabled(true))
        manager.dispatch(AppAction.SetVideoCallsEnabled(true))
        val owner = requiredArg("fixture_owner")
        manager.dispatch(AppAction.AcceptInvite(requiredArg("fixture_invite")))
        waitForState("control contact", timeoutMs = 60_000) {
            manager.state.value.chatList.firstOrNull { it.chatId == owner }
        }
        manager.dispatch(AppAction.SetMessageRequestAccepted(owner))
        manager.sendText(owner, "Control receiver ready")
        reportStatus("control_contact_created" to "true")
    }
}
