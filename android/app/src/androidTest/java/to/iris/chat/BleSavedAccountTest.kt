package to.iris.chat

import android.util.Base64
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction

/** Restore only the settings explicitly borrowed by the physical Bluetooth gate. */
@RunWith(AndroidJUnit4::class)
class BleSavedAccountTest : RealRelayHarnessBase() {
    @get:Rule
    override val activityRule = ActivityScenarioRule(MainActivity::class.java)

    @Test
    fun restore_preferences() {
        require(appPackageName() == "to.iris.chat.blegate")
        requiredArg("preserve_owner")
        requiredArg("preserve_devices")
        ensureLoggedIn()
        val saved = JSONObject(String(Base64.decode(requiredArg("saved_preferences"), Base64.NO_WRAP), Charsets.UTF_8))
        val relays = saved.getJSONArray("relays").let { array ->
            (0 until array.length()).map { array.getString(it) }
        }
        fun enabled(key: String): Boolean = saved.getInt(key).also { require(it in 0..1) } == 1
        val receipts = enabled("send_read_receipts")
        val nearby = enabled("nearby_enabled")
        val bluetooth = enabled("nearby_bluetooth_enabled")
        val lan = enabled("nearby_lan_enabled")
        appManager().dispatch(AppAction.SetReadReceiptsEnabled(receipts))
        appManager().dispatch(AppAction.SetNearbyBluetoothEnabled(bluetooth))
        appManager().dispatch(AppAction.SetNearbyLanEnabled(lan))
        appManager().dispatch(AppAction.SetNearbyEnabled(nearby))
        appManager().dispatch(AppAction.SetNostrRelays(relays))
        waitForState("restored Bluetooth test preferences", timeoutMs = 30_000) {
            appManager().state.value.preferences.takeIf {
                it.sendReadReceipts == receipts && it.nearbyEnabled == nearby &&
                    it.nearbyBluetoothEnabled == bluetooth && it.nearbyLanEnabled == lan &&
                    it.nostrRelayUrls == relays
            }
        }
        reportStatus("saved_preferences_restored" to "true")
    }
}
