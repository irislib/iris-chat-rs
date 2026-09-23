package to.iris.chat

import android.os.Bundle
import android.os.SystemClock
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.FfiApp

/** A real remote signer answers the emitted connection URI through the supplied local relay.
 * Run only on a dedicated test installation; the first phase resets its test account.
 * The runner must keep connection tokens private and invoke restart in a fresh app process.
 */
@RunWith(AndroidJUnit4::class)
class RemoteSignerE2eTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val arguments get() = InstrumentationRegistry.getArguments()
    private val manager get() = (compose.activity.application as IrisChatApp).container.appManager
    private val checkpoint get() = File(instrumentation.targetContext.filesDir, "remote-signer-checkpoint.json")

    @Test
    fun authorizes_device_and_exchanges_messages() {
        val relay = relayUrl()
        waitFor("account bootstrap") { manager.bootstrapState.value !is AccountBootstrapState.Loading }
        manager.resetForUiTestsBlocking()
        waitFor("welcome") { hasTag("welcomeRestoreAction") }
        waitFor("test account cleared") { !runBlocking { manager.hasPersistedDeviceSecret() } }
        manager.dispatch(AppAction.SetNostrRelays(listOf(relay)))
        waitFor("local message server") { manager.state.value.preferences.nostrRelayUrls == listOf(relay) }
        compose.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true).performClick()
        waitFor("signer action") { hasTag("restoreSignerAction") }
        compose.onNodeWithTag("restoreSignerAction", useUnmergedTree = true).performClick()
        waitFor("signer screen") { hasTag("remoteSignerScreen") }
        val bunker = arguments.getString("nip46_bunker").orEmpty()
        if (bunker.isNotEmpty()) {
            compose.onNodeWithTag("remoteSignerPasteLink", useUnmergedTree = true).performScrollTo().performClick()
            waitFor("signer link input") { hasTag("remoteSignerLinkInput") }
            compose.onNodeWithTag("remoteSignerLinkInput", useUnmergedTree = true).performTextReplacement(bunker)
            compose.onNodeWithTag("remoteSignerConnectAction", useUnmergedTree = true).performScrollTo().performClick()
        } else {
            waitFor("remote connection code") {
                manager.state.value.remoteSignerLogin?.connectionUri?.startsWith("nostrconnect://") == true
            }
            instrumentation.sendStatus(0, Bundle().apply {
                putString("nip46_connection_uri", manager.state.value.remoteSignerLogin!!.connectionUri)
            })
        }
        waitFor("signer approval", 180_000) {
            manager.state.value.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
        }
        val account = manager.state.value.account!!
        assertFalse(account.hasOwnerSigningAuthority)
        assertNull(runBlocking { manager.exportOwnerNsec() })
        waitFor("device key stored") { runBlocking { manager.hasPersistedDeviceSecret() } }
        waitFor("signer connection closed") { manager.state.value.remoteSignerLogin == null }
        assertFalse(manager.signer.busy.value)
        exchangeMessages(relay, account.publicKeyHex)
        checkpoint.writeText(JSONObject().put("owner", account.publicKeyHex).put("device", account.devicePublicKeyHex).toString())
        instrumentation.sendStatus(0, Bundle().apply { putString("nip46_stage", "authorized_and_messages_exchanged") })
    }

    @Test
    fun restores_without_the_signer() {
        val relay = relayUrl()
        assumeTrue("Run after authorization in a fresh process", arguments.getString("nip46_restart") == "1")
        val saved = JSONObject(checkpoint.readText())
        waitFor("restored device authorization") {
            manager.state.value.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
        }
        val account = manager.state.value.account!!
        assertEquals(saved.getString("owner"), account.publicKeyHex)
        assertEquals(saved.getString("device"), account.devicePublicKeyHex)
        assertFalse(account.hasOwnerSigningAuthority)
        assertNull(runBlocking { manager.exportOwnerNsec() })
        assertNull(manager.state.value.remoteSignerLogin)
        assertFalse(manager.signer.busy.value)
        exchangeMessages(relay, account.publicKeyHex)
    }

    private fun relayUrl(): String {
        val relay = arguments.getString("nip46_relay").orEmpty()
        assumeTrue("Explicit isolated remote-signer test required", arguments.getString("nip46_e2e") == "1")
        assumeTrue("Use a local test message server", relay.startsWith("ws://127.0.0.1:"))
        return relay
    }

    private fun hasTag(tag: String): Boolean = runCatching {
        compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }.getOrDefault(false)

    private fun exchangeMessages(relay: String, owner: String) {
        val directory = File(instrumentation.targetContext.cacheDir, "remote-signer-peer-${UUID.randomUUID()}").apply { mkdirs() }
        val peer = FfiApp(directory.absolutePath, "", "remote-signer-e2e")
        try {
            peer.dispatch(AppAction.SetNostrRelays(listOf(relay)))
            peer.dispatch(AppAction.CreateAccount("Remote signer test peer"))
            waitFor("independent peer") { peer.state().account != null }
            val peerKey = peer.state().account!!.publicKeyHex
            manager.dispatch(AppAction.CreateChat(peerKey))
            waitFor("conversation") { manager.state.value.chatList.any { it.chatId == peerKey } }
            val outgoing = "Remote signer outgoing ${UUID.randomUUID()}"
            manager.dispatch(AppAction.SendMessage(peerKey, outgoing))
            waitFor("outgoing message received") {
                peer.chatSnapshot(owner, 50u)?.messages?.any { it.body == outgoing && !it.isOutgoing } == true
            }
            val incoming = "Remote signer incoming ${UUID.randomUUID()}"
            peer.dispatch(AppAction.SendMessage(owner, incoming))
            waitFor("incoming message received") {
                manager.state.value.currentChat?.messages?.any { it.body == incoming && !it.isOutgoing } == true
            }
            assertTrue(runBlocking { manager.hasPersistedDeviceSecret() })
        } finally {
            peer.shutdown()
            peer.close()
            directory.deleteRecursively()
        }
    }

    private fun waitFor(label: String, timeoutMs: Long = 60_000, predicate: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            if (predicate()) return
            SystemClock.sleep(100)
        }
        error("Timed out: $label")
    }
}
