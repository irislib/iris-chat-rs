package to.iris.chat.calls

import android.Manifest
import android.os.Bundle
import android.os.SystemClock
import android.system.Os
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
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
import org.json.JSONObject
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

/** Real native codecs and a FIPS echo peer, with a fresh account and both ends denied WAN. */
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
        check(instrumentation is NativeCallTestRunner)
        listOf(relay!!, seed!!).forEach { check(URI(it).let { url -> url.scheme == "ws" && url.host == "127.0.0.1" }) }
        val context = instrumentation.targetContext
        listOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.CAMERA).forEach {
            instrumentation.uiAutomation.grantRuntimePermission(context.packageName, it)
        }
        val screen = mutableStateOf<@Composable () -> Unit>({ Text("Local voice and video call test") })
        compose.setContent { screen.value() }
        compose.waitForIdle()
        assertTrue("Test app must be denied Internet access", runCatching {
            Socket().use { it.connect(InetSocketAddress("1.1.1.1", 443), 600) }
        }.isFailure)
        val directory = File(context.cacheDir, "native-call-e2e-${UUID.randomUUID()}").apply { mkdirs() }
        val remoteEnded = File(directory, "remote-ended")
        status("nativeCallControlPath", remoteEnded.absolutePath)
        val environment = mapOf("IRIS_DEMO_RELAYS" to relay, "IRIS_FIPS_WEBSOCKET_SEED_URLS" to seed,
            "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR" to "", "IRIS_CHAT_FIPS_UDP_BIND_ADDR" to "127.0.0.1:0")
        val previous = environment.mapValues { Os.getenv(it.key) }
        environment.forEach { (key, value) -> Os.setenv(key, value, true) }
        var app: FfiApp? = null
        var audio: CallAudio? = null
        var video: CallVideoMedia? = null
        var route: CallAudioRoute? = null
        val audioSink = AtomicReference<CallAudio?>(null)
        val videoSink = AtomicReference<CallVideoMedia?>(null)
        val audioSent = AtomicInteger(); val audioReceived = AtomicInteger(); val played = AtomicInteger()
        val nonzeroPlayed = AtomicInteger(); val videoSent = AtomicInteger(); val videoReceived = AtomicInteger()
        val decoded = AtomicInteger(); val pixels = AtomicInteger(); val width = AtomicInteger(); val height = AtomicInteger()
        val failed = AtomicReference<String?>(null); val callId = AtomicReference<String?>(null)
        val hashes = ConcurrentHashMap.newKeySet<String>()
        try {
            val ffi = FfiApp(directory.absolutePath, "", "native-call-e2e").also { app = it }
            ffi.listenForUpdates(object : AppReconciler {
                override fun reconcile(update: AppUpdate) {
                    if (update !is AppUpdate.CallMedia || update.callId != callId.get()) return
                    if (!hashes.contains(hash(update.data))) { failed.set("Echo changed a captured frame"); return }
                    when (update.kind.toInt()) {
                        1 -> { audioReceived.incrementAndGet(); audioSink.get()?.receive(update.sequence, update.data) }
                        2 -> { videoReceived.incrementAndGet(); videoSink.get()?.receive(update.sequence, update.data, update.timestampUs, update.keyFrame) }
                    }
                }
            })
            ffi.dispatch(AppAction.SetNostrRelays(listOf(relay)))
            ffi.dispatch(AppAction.SetNearbyLanEnabled(true))
            ffi.dispatch(AppAction.CreateAccount("Android call test"))
            await("fresh account") { ffi.state().account != null }
            status("nativeCallOwner", checkNotNull(ffi.state().account).publicKeyHex)
            ffi.dispatch(AppAction.AcceptInvite(invite!!))
            await("accepted direct chat", 45_000) { ffi.state().chatList.any { it.chatId == peer } }
            ffi.dispatch(AppAction.SetMessageRequestAccepted(peer!!))
            ffi.dispatch(AppAction.SendMessage(peer, "Call setup"))
            await("authenticated contact devices", 45_000) {
                ffi.peerProfileDebug(peer)?.let { it.rosterDeviceCount > 0uL && it.activeSessionCount > 0uL } == true
            }
            status("nativeCallPhase", "contact_ready")
            await("message server stopped", 15_000) {
                runCatching { Socket().use { it.connect(InetSocketAddress("127.0.0.1", URI(relay).port), 250) } }.isFailure
            }
            ffi.dispatch(AppAction.StartCall(peer, true))
            if (voiceOnly) ffi.dispatch(AppAction.SetCallMuted(true))
            await("accepted call", 45_000) {
                val call = ffi.state().call
                check(call?.phase != "ended") { "Call ended: ${call?.endReason}" }
                call?.phase == "connected"
            }
            val connected = checkNotNull(ffi.state().call)
            assertEquals(!voiceOnly, connected.videoCapable)
            callId.set(connected.callId)
            route = CallAudioRoute(context) { failed.set("Audio focus lost") }.also { it.start(false) }
            audio = CallAudio(context, send = { bytes, timestamp ->
                hashes.add(hash(bytes)); audioSent.incrementAndGet()
                ffi.dispatch(AppAction.SendCallMedia(connected.callId, 1u, timestamp, false, bytes))
            }, failed = { failed.set("Native audio failed") }, played = { samples ->
                played.incrementAndGet()
                if (samples.any { it.toInt() != 0 }) nonzeroPlayed.incrementAndGet()
            })
            audioSink.set(audio); audio!!.start(voiceOnly)
            if (!voiceOnly) {
                video = CallVideoMedia(context, send = { bytes, timestamp, key ->
                    hashes.add(hash(bytes)); videoSent.incrementAndGet()
                    ffi.dispatch(AppAction.SendCallMedia(connected.callId, 2u, timestamp, key, bytes))
                }, requestKey = { ffi.dispatch(AppAction.RequestCallKeyFrame(connected.callId)) },
                    cameraFailed = { failed.set("Native camera failed") }, decoded = { decoded.incrementAndGet() })
                video!!.remoteVideo.add { frame ->
                    val count = frame.rotatedWidth * frame.rotatedHeight
                    pixels.accumulateAndGet(count, ::maxOf); width.set(frame.rotatedWidth); height.set(frame.rotatedHeight)
                }
                videoSink.set(video)
            }
            ffi.dispatch(AppAction.SetCallMediaConnected(connected.callId, true))
            if (voiceOnly) {
                val until = SystemClock.elapsedRealtime() + 31_000
                while (SystemClock.elapsedRealtime() < until) {
                    assertEquals("An initially muted call remains connected", "connected", ffi.state().call?.phase)
                    SystemClock.sleep(250)
                }
                assertEquals("Initially muted microphone sends nothing", 0, audioSent.get())
                status("nativeMutedCall", "connected_without_capture_for_31_seconds")
                ffi.dispatch(AppAction.SetCallMuted(false)); audio!!.muted = false
            }
            fun sync() {
                val call = checkNotNull(ffi.state().call)
                check(call.phase == "connected") { "Call ended: ${call.endReason}" }
                check(failed.get() == null) { failed.get()!! }
                video?.update(call.video, CallQuality(ffi.state().preferences.callQuality, call.maxBitrateBps.toInt()),
                    call.targetBitrateBps.toInt(), call.keyFrameGeneration)
            }
            await("native Opus and H.264 echoed over FIPS and decoded", 45_000) {
                sync()
                audioReceived.get() >= 30 && played.get() >= 20 && (voiceOnly || decoded.get() >= 20)
            }
            if (!voiceOnly) {
                assertTrue("Video exceeds the old preview limit", pixels.get() > 320 * 240)
                ffi.dispatch(AppAction.SetCallQuality("custom", 350_000u))
                await("lower bandwidth video still decodes") {
                    sync()
                    ffi.state().call?.maxBitrateBps == 350_000u && width.get() * height.get() <= 640 * 480
                }
                ffi.dispatch(AppAction.SetCallQuality("auto", 350_000u))
                await("higher quality video returns", 30_000) { sync(); width.get() * height.get() > 640 * 480 }
            }
            compose.runOnUiThread { screen.value = { to.iris.chat.ui.theme.IrisChatTheme {
                CallSurface(checkNotNull(ffi.state().call).copy(peerName = "Alex"), true, true,
                    video?.remoteVideo, video?.localVideo, null, false, System.currentTimeMillis() / 1_000,
                    { _, next -> next() }, ffi::dispatch, {}, {})
            } } }
            compose.waitForIdle(); SystemClock.sleep(300)
            val screenshot = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
            val screenshots = checkNotNull(context.getExternalFilesDir("screenshots")).apply { mkdirs() }
            File(screenshots, if (voiceOnly) "native-fips-voice.png" else "native-fips-video.png").outputStream().use {
                screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
            }
            screenshot.recycle()
            audio!!.muted = true
            video?.update(false, CallQuality(), 2_000_000, 0u)
            SystemClock.sleep(150)
            val stoppedAudio = audioSent.get(); val stoppedVideo = videoSent.get()
            SystemClock.sleep(200)
            assertEquals("Mute stops encoded microphone frames", stoppedAudio, audioSent.get())
            assertEquals("Camera off stops encoded video frames", stoppedVideo, videoSent.get())
            ffi.dispatch(AppAction.SetCallMuted(true)); ffi.dispatch(AppAction.SetCallVideoEnabled(false))
            videoSink.set(null); audioSink.set(null); video?.close(); video = null; audio?.close(); audio = null
            ffi.dispatch(AppAction.EndCall(connected.callId))
            await("call ended") { ffi.state().call?.phase == "ended" }
            status("nativeCallPhase", "hangup_sent")
            await("remote hangup confirmed", 10_000) { remoteEnded.isFile }
            assertEquals(null, failed.get())
            if (voiceOnly) assertEquals(0, videoSent.get())
            status("nativeCallResult", JSONObject().put("audio_codec", "Opus/48000").put("video_codec", "H264/AnnexB")
                .put("audio_sent", audioSent.get()).put("audio_received", audioReceived.get()).put("audio_played", played.get())
                .put("nonzero_playout", nonzeroPlayed.get()).put("video_sent", videoSent.get()).put("video_received", videoReceived.get())
                .put("video_decoded", decoded.get()).put("max_video_pixels", pixels.get()).put("remote_hangup", true).toString())
        } catch (error: Throwable) {
            val evidence = checkNotNull(context.getExternalFilesDir("call-tests")).apply { mkdirs() }
            app?.let { File(evidence, "native-peer-log.json").writeText(it.exportSupportBundleJson()) }
            status("nativeCallError", "${error.message}; phase=${app?.state()?.call?.phase}; audio=${audioReceived.get()}; video=${videoReceived.get()}; decoded=${decoded.get()}")
            throw error
        } finally {
            compose.runOnUiThread { screen.value = { Text("Call test complete") } }; compose.waitForIdle()
            videoSink.set(null); audioSink.set(null); video?.close(); audio?.close(); route?.close()
            app?.shutdown(); app?.close()
            previous.forEach { (key, value) -> if (value == null) Os.unsetenv(key) else Os.setenv(key, value, true) }
            directory.deleteRecursively()
        }
    }

    private fun status(key: String, value: String) = InstrumentationRegistry.getInstrumentation().sendStatus(0, Bundle().apply { putString(key, value) })
    private fun hash(bytes: ByteArray): String = android.util.Base64.encodeToString(MessageDigest.getInstance("SHA-256").digest(bytes), android.util.Base64.NO_WRAP)
    private fun await(description: String, timeout: Long = 30_000, ready: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeout
        while (!ready()) { check(SystemClock.elapsedRealtime() < deadline) { "Timed out waiting for $description" }; SystemClock.sleep(40) }
    }
}
