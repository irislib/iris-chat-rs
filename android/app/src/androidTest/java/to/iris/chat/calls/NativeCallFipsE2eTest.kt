package to.iris.chat.calls

import android.Manifest
import android.graphics.BitmapFactory
import android.os.Bundle
import android.os.SystemClock
import android.system.Os
import androidx.compose.material3.Text
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.net.URI
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.FfiApp

/** Requires the opt-in runner and local native echo fixture; no installed account data is used. */
@RunWith(AndroidJUnit4::class)
class NativeCallFipsE2eTest {
    @get:Rule val compose = createComposeRule()

    @Test fun nativeMediaRoundTripsThroughFipsWithMessageServerStopped() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val args = InstrumentationRegistry.getArguments()
        val relay = args.getString("call_relay")
        val seed = args.getString("call_fips_seed")
        val invite = args.getString("call_invite")
        val peer = args.getString("call_peer_owner")
        val voiceOnly = args.getString("call_answer_voice") == "1"
        assumeTrue("Run with the local native call fixture", listOf(relay, seed, invite, peer).all { !it.isNullOrBlank() })
        check(instrumentation is NativeCallTestRunner) { "Use NativeCallTestRunner to protect installed account data" }
        listOf(relay!!, seed!!).forEach {
            check(URI(it).scheme == "ws" && URI(it).host == "127.0.0.1") { "Only forwarded loopback endpoints are allowed" }
        }
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.CAMERA)
        compose.setContent { Text("Local voice and video call test") }
        compose.waitForIdle()
        assertTrue("The test runner must block Internet access for this app", runCatching {
            Socket().use { it.connect(InetSocketAddress("1.1.1.1", 443), 600) }
        }.isFailure)
        val directory = File(context.cacheDir, "native-call-e2e-${UUID.randomUUID()}").apply { mkdirs() }
        val remoteEnded = File(directory, "remote-ended")
        status("nativeCallControlPath", remoteEnded.absolutePath)
        val environment = mapOf("IRIS_DEMO_RELAYS" to relay, "IRIS_FIPS_WEBSOCKET_SEED_URLS" to seed,
            "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR" to "")
        val previous = environment.mapValues { Os.getenv(it.key) }
        environment.forEach { (key, value) -> Os.setenv(key, value, true) }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        var app: FfiApp? = null
        var audio: CallAudio? = null
        val audioSink = AtomicReference<CallAudio?>(null)
        var camera: CallCamera? = null
        val audioSent = AtomicInteger()
        val videoSent = AtomicInteger()
        val audioReceived = AtomicInteger()
        val videoReceived = AtomicInteger()
        val audioPlayed = AtomicInteger()
        val failed = AtomicReference<String?>(null)
        val callId = AtomicReference<String?>(null)
        val hashes = ConcurrentHashMap.newKeySet<String>()
        try {
            val ffi = FfiApp(directory.absolutePath, "", "native-call-e2e")
            app = ffi
            ffi.listenForUpdates(object : AppReconciler {
                override fun reconcile(update: AppUpdate) {
                    if (update !is AppUpdate.CallMedia || update.callId != callId.get()) return
                    if (!hashes.contains(hash(update.data))) { failed.set("Echo changed a captured frame"); return }
                    when (update.kind.toInt()) {
                        1 -> {
                            if (update.data.size != CallAudio.FRAME_BYTES) failed.set("Invalid PCM frame")
                            audioReceived.incrementAndGet()
                            audioSink.get()?.receive(update.data)
                        }
                        2 -> {
                            val bitmap = BitmapFactory.decodeByteArray(update.data, 0, update.data.size)
                            if (bitmap == null || bitmap.width > 320 || bitmap.height > 240) failed.set("Invalid JPEG frame")
                            bitmap?.recycle()
                            videoReceived.incrementAndGet()
                        }
                    }
                }
            })
            ffi.dispatch(AppAction.SetNostrRelays(listOf(relay)))
            ffi.dispatch(AppAction.CreateAccount("Android call test"))
            await("fresh account", 30_000) { ffi.state().account != null }
            val account = checkNotNull(ffi.state().account)
            status("nativeCallOwner", account.publicKeyHex)
            ffi.dispatch(AppAction.SetNearbyLanEnabled(true))
            ffi.dispatch(AppAction.AcceptInvite(invite!!))
            await("accepted direct chat", 45_000) { ffi.state().chatList.any { it.chatId == peer } }
            ffi.dispatch(AppAction.SetMessageRequestAccepted(peer!!))
            ffi.dispatch(AppAction.SendMessage(peer, "Call setup"))
            await("authenticated contact devices", 45_000) {
                ffi.peerProfileDebug(peer)?.let { it.rosterDeviceCount > 0uL && it.activeSessionCount > 0uL } == true
            }
            // The host runner stops the loopback message server at this handshake.
            status("nativeCallPhase", "contact_ready")
            val relayUri = URI(relay)
            await("message server stopped", 15_000) {
                !runCatching { Socket().use { socket ->
                    socket.connect(InetSocketAddress(relayUri.host, relayUri.port), 250)
                    socket.soTimeout = 250
                    socket.getOutputStream().write(("GET / HTTP/1.1\r\nHost: localhost\r\n" +
                        "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n" +
                        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").toByteArray())
                    socket.getInputStream().read() >= 0
                } }.getOrDefault(false)
            }
            ffi.dispatch(AppAction.StartCall(peer, true))
            await("connected video call", 45_000) {
                val call = ffi.state().call
                check(call?.phase != "ended") { "Call ended: ${call?.endReason}" }
                call?.phase == "connected"
            }
            val connected = checkNotNull(ffi.state().call)
            assertEquals(!voiceOnly, connected.videoCapable)
            assertEquals(!voiceOnly, connected.video)
            callId.set(connected.callId)
            status("nativeCallPhase", if (voiceOnly) "voice_connected" else "video_connected")
            audio = CallAudio(context, scope, send = { bytes ->
                hashes.add(hash(bytes)); audioSent.incrementAndGet()
                ffi.dispatch(AppAction.SendCallMedia(connected.callId, 1u, bytes))
            }, failed = { failed.set("Native audio failed") }, played = { audioPlayed.incrementAndGet() })
            audioSink.set(audio)
            audio!!.start(false)
            if (!voiceOnly) {
                camera = CallCamera(context, send = { bytes ->
                    hashes.add(hash(bytes)); videoSent.incrementAndGet()
                    ffi.dispatch(AppAction.SendCallMedia(connected.callId, 2u, bytes))
                }, preview = {}, failed = { failed.set("Native camera failed") })
                camera!!.start()
            }
            await("captured PCM and JPEG echoed over FIPS and played", 30_000) {
                check(failed.get() == null) { failed.get()!! }
                audioReceived.get() >= 30 && audioPlayed.get() >= 20 && (voiceOnly || videoReceived.get() >= 3)
            }
            camera?.close(); camera = null
            audio!!.muted = true
            ffi.dispatch(AppAction.SetCallMuted(true))
            ffi.dispatch(AppAction.SetCallVideoEnabled(false))
            await("mute and camera state", 5_000) { ffi.state().call?.let { it.muted && !it.video } == true }
            audioSink.set(null); audio!!.close(); audio = null
            ffi.dispatch(AppAction.EndCall(connected.callId))
            await("call ended", 5_000) { ffi.state().call?.phase == "ended" }
            status("nativeCallPhase", "hangup_sent")
            await("remote hangup confirmed", 10_000) { remoteEnded.isFile }
            assertEquals(null, failed.get())
            if (voiceOnly) { assertEquals(0, videoSent.get()); assertEquals(0, videoReceived.get()) }
            status("nativeCallResult", "audio_sent=${audioSent.get()},audio_received=${audioReceived.get()},audio_played=${audioPlayed.get()},video_sent=${videoSent.get()},video_received=${videoReceived.get()}")
        } catch (error: Throwable) {
            status("nativeCallError", "${error.message}; toast=${app?.state()?.toast}; peer=${app?.peerProfileDebug(peer!!)}")
            throw error
        } finally {
            audioSink.set(null); camera?.close(); audio?.close()
            app?.shutdown(); app?.close()
            scope.cancel()
            previous.forEach { (key, value) -> if (value == null) Os.unsetenv(key) else Os.setenv(key, value, true) }
            directory.deleteRecursively()
        }
    }

    private fun status(key: String, value: String) = InstrumentationRegistry.getInstrumentation()
        .sendStatus(0, Bundle().apply { putString(key, value) })

    private fun hash(bytes: ByteArray): String = android.util.Base64.encodeToString(
        MessageDigest.getInstance("SHA-256").digest(bytes), android.util.Base64.NO_WRAP)

    private fun await(description: String, timeout: Long, ready: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeout
        while (!ready()) {
            check(SystemClock.elapsedRealtime() < deadline) { "Timed out waiting for $description" }
            SystemClock.sleep(40)
        }
    }
}
