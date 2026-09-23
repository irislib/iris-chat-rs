package to.iris.chat

import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.FfiApp

/** Run via scripts/test-android-signer on an isolated emulator with the fixture APK. */
@RunWith(AndroidJUnit4::class)
class Nip55SignerE2eTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()
    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val arguments get() = InstrumentationRegistry.getArguments()
    private val manager get() = (compose.activity.application as IrisChatApp).container.appManager
    private val checkpoint get() = File(instrumentation.targetContext.filesDir, "signer-e2e-checkpoint.json")

    @Test
    fun amber_login_with_manual_approval() {
        assumeTrue("Only run with an explicitly prepared Amber test identity", arguments.getString("amber_e2e") == "1")
        val relay = relayUrl()
        reset(relay)
        startThroughRestoreScreen()
        instrumentation.sendStatus(0, Bundle().apply { putString("signer_stage", "waiting_for_manual_amber_approval") })
        waitFor("manual Amber approval and authorized account", timeoutMs = 180_000) {
            manager.state.value.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
        }
        val account = manager.state.value.account!!
        assertFalse(account.hasOwnerSigningAuthority)
        assertNull(runBlocking { manager.exportOwnerNsec() })
        waitFor("persisted local device key") { runBlocking { manager.hasPersistedDeviceSecret() } }
        exchangeWithIndependentPeer(relay, account.publicKeyHex)
        assertFalse(manager.signer.busy.value)
        instrumentation.sendStatus(0, Bundle().apply { putString("signer_stage", "amber_authorized_and_messages_exchanged") })
    }

    @Test
    fun authorizes_preserving_devices_and_messages_without_more_signer_requests() {
        val relay = relayUrl()
        reset(relay)
        val owner = control("reset").getString("pubkey")!!
        val existingDevice = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"
        val id = UUID.randomUUID().toString()
        val oldTimestamp = System.currentTimeMillis() / 1000 - 60
        val tags = JSONArray()
            .put(JSONArray(listOf("d", id)))
            .put(JSONArray(listOf("i", id, "subject")))
            .put(JSONArray(listOf("p", owner)))
            .put(JSONArray(listOf("p", existingDevice)))
            .put(JSONArray(listOf("type", "app_keys_roster_snapshot")))
            .put(JSONArray(listOf("schema", "1")))
            .put(JSONArray(listOf("owner_pubkey", owner)))
            .put(JSONArray(listOf("device", existingDevice, oldTimestamp.toString())))
            .put(JSONArray(listOf("encrypted_device_labels", "opaque-fixture-labels")))
        val oldRoster = JSONObject().put("pubkey", owner).put("created_at", oldTimestamp)
            .put("kind", 37368).put("tags", tags).put("content", "")
        val signedOldRoster = control("sign_fixture", oldRoster.toString()).getString("event")!!
        publish(relay, signedOldRoster)
        prepareOfflineSibling(relay, owner, existingDevice)

        startThroughRestoreScreen()
        approveNativeSignerRequest("get_public_key")
        approveNativeSignerRequest("sign_event")
        waitFor("authorized signer account") {
            manager.state.value.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
        }
        val account = manager.state.value.account!!
        stage("signer_authorized")
        assertEquals(owner, account.publicKeyHex)
        assertFalse(account.hasOwnerSigningAuthority)
        assertNull(runBlocking { manager.exportOwnerNsec() })
        waitFor("persisted local device key") { runBlocking { manager.hasPersistedDeviceSecret() } }
        val signed = JSONObject(control("status").getString("last_event")!!)
        val returnedTags = signed.getJSONArray("tags")
        assertTrue(hasTag(returnedTags, "device", existingDevice))
        assertTrue(hasTag(returnedTags, "device", account.devicePublicKeyHex))
        assertTrue(hasTag(returnedTags, "encrypted_device_labels", "opaque-fixture-labels"))
        assertEquals(2, control("status").getInt("request_count"))

        exchangeWithIndependentPeer(relay, owner)
        assertEquals("Messaging must only use the local device key", 2, control("status").getInt("request_count"))
        checkpoint.writeText(JSONObject().put("owner", owner).put("device", account.devicePublicKeyHex).toString())
    }

    @Test
    fun rejects_signer_denial_without_persisting_an_account() {
        reset(relayUrl())
        control("reset")
        startThroughRestoreScreen()
        approveNativeSignerRequest("get_public_key")
        waitFor("sign request reaches separate app") { control("status").getString("last_method") == "sign_event" }
        clickNative("Reject")
        waitFor("denied login settles") { !manager.signer.busy.value && !manager.state.value.busy.restoringSession }
        assertNull(manager.state.value.account)
        assertFalse(runBlocking { manager.hasPersistedDeviceSecret() })
    }

    @Test
    fun rejects_a_valid_signature_for_a_different_event() {
        reset(relayUrl())
        control("reset")
        control("mode", "wrong_event")
        startThroughRestoreScreen()
        waitFor("mutated event returned") { control("status").getInt("sign_event_count") == 1 }
        waitFor("invalid authorization settles") { !manager.signer.busy.value && !manager.state.value.busy.restoringSession }
        assertNull(manager.state.value.account)
        assertFalse(runBlocking { manager.hasPersistedDeviceSecret() })
    }

    @Test
    fun rejects_a_response_with_the_wrong_request_id() {
        reset(relayUrl())
        control("reset")
        control("mode", "wrong_id")
        startThroughRestoreScreen()
        waitFor("wrong response returned") { control("status").getInt("get_public_key_count") == 1 }
        waitFor("invalid response settles") { !manager.signer.busy.value }
        assertNull(manager.state.value.account)
        assertEquals(0, control("status").getInt("sign_event_count"))
        assertFalse(runBlocking { manager.hasPersistedDeviceSecret() })
    }

    @Test
    fun restores_after_process_restart_without_calling_the_signer() {
        assumeTrue("The runner invokes this after the successful login phase", arguments.getString("signer_restart") == "1")
        val saved = JSONObject(checkpoint.readText())
        waitFor("account restored from device-only secure storage") {
            manager.state.value.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
        }
        val account = manager.state.value.account!!
        assertEquals(saved.getString("owner"), account.publicKeyHex)
        assertEquals(saved.getString("device"), account.devicePublicKeyHex)
        assertNull(runBlocking { manager.exportOwnerNsec() })
        assertFalse(account.hasOwnerSigningAuthority)
        assertEquals(2, control("status").getInt("request_count"))
        exchangeWithIndependentPeer(relayUrl(), account.publicKeyHex)
        assertEquals(2, control("status").getInt("request_count"))
    }

    private fun relayUrl(): String {
        val relay = arguments.getString("signer_relay").orEmpty()
        assumeTrue("Use scripts/test-android-signer with a local relay", relay.startsWith("ws://"))
        return relay
    }

    private fun reset(relay: String) {
        waitFor("initial credential restore settles") { manager.bootstrapState.value !is AccountBootstrapState.Loading }
        manager.resetForUiTestsBlocking()
        waitFor("welcome") { hasTag("welcomeRestoreAction") }
        waitFor("test credentials cleared") { !runBlocking { manager.hasPersistedDeviceSecret() } }
        manager.dispatch(AppAction.SetNostrRelays(listOf(relay)))
        waitFor("local test relay selected") { manager.state.value.preferences.nostrRelayUrls == listOf(relay) }
    }

    private fun startThroughRestoreScreen() {
        stage("opening_restore")
        compose.onNodeWithTag("welcomeRestoreAction", useUnmergedTree = true).performClick()
        waitFor("signer login button") { hasTag("restoreSignerAction") }
        stage("opening_signer")
        compose.onNodeWithTag("restoreSignerAction", useUnmergedTree = true).performClick()
        stage("signer_opened")
    }

    private fun approveNativeSignerRequest(method: String) {
        stage("waiting_for_$method")
        waitFor("$method reaches separate signer app") { control("status").getString("last_method") == method }
        stage("approving_$method")
        clickNative("Approve")
    }

    private fun clickNative(label: String) {
        waitFor("native signer $label button") {
            val root = instrumentation.uiAutomation.rootInActiveWindow ?: return@waitFor false
            if (root.packageName?.toString() != "to.iris.test.signer") return@waitFor false
            root.findAccessibilityNodeInfosByText(label)
                .firstOrNull { it.text?.toString()?.equals(label, ignoreCase = true) == true && it.isClickable }
                ?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
        }
        stage("clicked_${label.lowercase()}")
    }

    private fun stage(label: String) {
        instrumentation.sendStatus(0, Bundle().apply { putString("signer_stage", label) })
    }

    private fun hasTag(tag: String): Boolean = runCatching {
        compose.onAllNodesWithTag(tag, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }.getOrDefault(false)

    private fun hasTag(tags: JSONArray, name: String, value: String): Boolean =
        (0 until tags.length()).any { index ->
            tags.getJSONArray(index).let { it.optString(0) == name && it.optString(1) == value }
        }

    private fun control(method: String, arg: String? = null): Bundle =
        instrumentation.targetContext.contentResolver.call(Uri.parse("content://to.iris.test.signer.control"), method, arg, null)
            ?: error("Install the :test-signer debug APK")

    private fun exchangeWithIndependentPeer(relay: String, owner: String) {
        val directory = File(instrumentation.targetContext.cacheDir, "signer-peer-${UUID.randomUUID()}").apply { mkdirs() }
        val peer = FfiApp(directory.absolutePath, "", "signer-e2e")
        try {
            peer.dispatch(AppAction.SetNostrRelays(listOf(relay)))
            peer.dispatch(AppAction.CreateAccount("Signer test peer"))
            waitFor("independent peer account") { peer.state().account != null }
            val peerKey = peer.state().account!!.publicKeyHex
            manager.dispatch(AppAction.CreateChat(peerKey))
            waitFor("signer chat created") { manager.state.value.chatList.any { it.chatId == peerKey } }
            val first = "from signer ${UUID.randomUUID()}"
            manager.dispatch(AppAction.SendMessage(peerKey, first))
            waitFor("peer receives signer device message") {
                peer.chatSnapshot(owner, 50u)?.messages?.any { it.body == first && !it.isOutgoing } == true
            }
            stage("peer_received_message")
            val reply = "to signer ${UUID.randomUUID()}"
            peer.dispatch(AppAction.SendMessage(owner, reply))
            waitFor("signer device receives reply") {
                manager.state.value.currentChat?.messages?.any { it.body == reply && !it.isOutgoing } == true
            }
            stage("signer_received_reply")
        } finally {
            peer.shutdown()
            peer.close()
            directory.deleteRecursively()
        }
    }

    private fun prepareOfflineSibling(relay: String, owner: String, device: String) {
        // An existing offline device has a published invitation. Use the real core to
        // establish that state before taking it offline; only its device key is used.
        val directory = File(instrumentation.targetContext.cacheDir, "signer-sibling-${UUID.randomUUID()}").apply { mkdirs() }
        val sibling = FfiApp(directory.absolutePath, "", "signer-e2e")
        val done = CountDownLatch(1)
        val client = OkHttpClient()
        val socket = client.newWebSocket(Request.Builder().url(relay).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                val filter = JSONObject().put("kinds", JSONArray(listOf(30078))).put("authors", JSONArray(listOf(device)))
                webSocket.send(JSONArray().put("REQ").put("sibling-invite").put(filter).toString())
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                val message = JSONArray(text)
                if (message.optString(0) == "EVENT" &&
                    hasTag(message.getJSONObject(2).getJSONArray("tags"), "ownerPublicKey", owner)
                ) done.countDown()
            }
        })
        try {
            sibling.dispatch(AppAction.SetNostrRelays(listOf(relay)))
            sibling.dispatch(AppAction.RestoreAccountBundle(null, owner, "0".repeat(63) + "3"))
            waitFor("existing device restored using only its device key") {
                sibling.state().account?.authorizationState == DeviceAuthorizationState.AUTHORIZED
            }
            assertTrue("Existing device published its invitation before going offline", done.await(30, TimeUnit.SECONDS))
            stage("offline_sibling_invite_published")
        } finally {
            socket.close(1000, "done")
            client.dispatcher.executorService.shutdown()
            client.connectionPool.evictAll()
            sibling.shutdown()
            sibling.close()
            directory.deleteRecursively()
        }
    }

    private fun publish(relay: String, event: String) {
        val done = CountDownLatch(1)
        var failure: String? = null
        val client = OkHttpClient()
        val socket = client.newWebSocket(Request.Builder().url(relay).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                webSocket.send(JSONArray().put("EVENT").put(JSONObject(event)).toString())
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                val message = JSONArray(text)
                if (message.optString(0) == "OK") {
                    if (!message.optBoolean(2)) failure = text
                    done.countDown()
                }
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                failure = t.message
                done.countDown()
            }
        })
        try {
            assertTrue("Local relay accepted existing roster", done.await(20, TimeUnit.SECONDS))
            assertNull(failure)
        } finally {
            socket.close(1000, "done")
            client.dispatcher.executorService.shutdown()
            client.connectionPool.evictAll()
        }
    }

    private fun waitFor(label: String, timeoutMs: Long = 90_000, predicate: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            if (predicate()) return
            SystemClock.sleep(100)
        }
        error("Timed out: $label")
    }
}
