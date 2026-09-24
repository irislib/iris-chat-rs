package to.iris.chat.calls

import android.Manifest
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.compose.material3.Text
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.sin
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.CallAudioCodec

@RunWith(AndroidJUnit4::class)
class NativeCallCodecTest {
    @get:Rule val compose = createComposeRule()

    @Test fun nativeCameraEncodesHdAndDecoderRecoversAfterFrameLoss() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue("Use the isolated native call runner", instrumentation is NativeCallTestRunner)
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.CAMERA)
        compose.setContent { Text("Native camera test") }; compose.waitForIdle()
        val media = AtomicReference<CallVideoMedia?>()
        val error = AtomicBoolean(); val frames = AtomicInteger(); val maxPixels = AtomicInteger()
        val sequence = AtomicInteger(); val generation = AtomicInteger(); val drop = AtomicBoolean()
        val dropped = AtomicBoolean(); val keys = AtomicInteger(); val receivedPixels = AtomicInteger()
        val encodedBytes = AtomicInteger()
        val repairs = Handler(Looper.getMainLooper())
        val firstAfterToggle = AtomicBoolean()
        val engine = CallVideoMedia(context, send = { bytes, timestamp, key ->
            if (firstAfterToggle.compareAndSet(true, false) && !key) error.set(true)
            encodedBytes.addAndGet(bytes.size)
            val number = sequence.getAndIncrement().toUInt()
            if (key) keys.incrementAndGet()
            if (number == 0u) repairs.postDelayed({ media.get()?.receive(number, bytes, timestamp, key) }, 120)
            else if (!key && drop.compareAndSet(true, false)) dropped.set(true)
            else media.get()?.receive(number, bytes, timestamp, key)
        }, requestKey = { generation.incrementAndGet() }, cameraFailed = { error.set(true) }, decoded = { frames.incrementAndGet() })
        media.set(engine)
        engine.remoteVideo.add {
            val size = it.rotatedWidth * it.rotatedHeight
            maxPixels.accumulateAndGet(size, ::maxOf); receivedPixels.set(size)
        }
        try {
            val began = SystemClock.elapsedRealtime()
            fun update(profile: String = "auto", cap: Int = 2_000_000) {
                check(!error.get()) { "Native camera/encoder failed" }
                engine.update(true, CallQuality(profile, cap), cap, generation.get().toUInt())
            }
            await("720p camera frames decoded") { update(); frames.get() >= 30 && maxPixels.get() >= 1280 * 720 }
            assertEquals("Repaired first IDR must unlock buffered video without losing its reference", 0, generation.get())
            val firstFrames = frames.get()
            val initialRequests = generation.get()
            drop.set(true)
            await("frame loss triggers key-frame recovery") {
                update(); dropped.get() && generation.get() > initialRequests && frames.get() >= firstFrames + 30
            }
            val beforeToggle = frames.get()
            firstAfterToggle.set(true)
            engine.update(false, CallQuality(), 2_000_000, generation.get().toUInt())
            update()
            await("fast camera toggle restarts with a new key frame") {
                update(); !firstAfterToggle.get() && frames.get() > beforeToggle + 10
            }
            await("lower cap scales decoded video") { update("custom", 350_000); receivedPixels.get() <= 640 * 480 }
            await("low bandwidth scales decoded video further") { update("custom", 150_000); receivedPixels.get() <= 320 * 240 }
            SystemClock.sleep(3000) // let the codec's rate-control window settle
            val lowBegan = SystemClock.elapsedRealtime()
            val lowBytes = encodedBytes.get(); val lowFrames = frames.get()
            SystemClock.sleep(5000)
            val lowSeconds = (SystemClock.elapsedRealtime() - lowBegan) / 1000.0
            val lowBps = (encodedBytes.get() - lowBytes) * 8 / lowSeconds
            val lowFps = (frames.get() - lowFrames) / lowSeconds
            assertTrue("Encoder must honor live 150 kbps target: $lowBps", lowBps < 230_000)
            assertTrue("Low bandwidth must keep decoding: $lowFps", lowFps >= 8)
            val beforeRecovery = frames.get()
            await("bandwidth recovery restores 720p and sustained decoding") {
                update(); receivedPixels.get() >= 1280 * 720 && frames.get() >= beforeRecovery + 60
            }
            instrumentation.sendStatus(0, Bundle().apply { putString("nativeBitrateResult", "target=150000,actual_bps=$lowBps,decoded_fps=$lowFps") })
            val stopped = sequence.get()
            engine.update(false, CallQuality(), 2_000_000, generation.get().toUInt())
            SystemClock.sleep(150)
            val settled = sequence.get(); SystemClock.sleep(200)
            assertEquals("Camera off sends no encoded frames", settled, sequence.get())
            assertTrue(stopped <= settled)
            instrumentation.sendStatus(0, Bundle().apply { putString("nativeCodecResult",
                "codec=H264/Baseline,decoded=${frames.get()},max_pixels=${maxPixels.get()},keyframes=${keys.get()},key_requests=${generation.get()},bytes=${encodedBytes.get()},elapsed_ms=${SystemClock.elapsedRealtime() - began}") })
        } finally { media.set(null); engine.close() }
    }

    @Test fun opusCapturePlaybackAndMuteUseNativeAudio() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue("Use the isolated native call runner", instrumentation is NativeCallTestRunner)
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        compose.setContent { Text("Native audio test") }; compose.waitForIdle()
        val error = AtomicBoolean(); val sent = AtomicInteger(); val nonzero = AtomicInteger()
        val route = CallAudioRoute(context) { error.set(true) }.apply { start(false) }
        val audio = CallAudio(context, { bytes, _ -> check(bytes.size in 1..1275); sent.incrementAndGet() },
            { error.set(true) }, { samples -> if (samples.any { it.toInt() != 0 }) nonzero.incrementAndGet() })
        try {
            audio.start(false)
            CallAudioCodec().use { source ->
                repeat(30) { index ->
                    val samples = List(960) { sample -> (sin((index * 960 + sample) * 2 * Math.PI * 440 / 48_000) * 4_000).toInt().toShort() }
                    audio.receive(index.toUInt(), source.encode(samples)); SystemClock.sleep(20)
                }
            }
            await("captured Opus and decoded audible samples") { check(!error.get()); sent.get() >= 20 && nonzero.get() >= 10 }
            val beforeToggle = sent.get()
            repeat(10) { audio.muted = true; SystemClock.sleep(1); audio.muted = false; SystemClock.sleep(1) }
            await("fast mute toggles resume fresh capture") { check(!error.get()); sent.get() > beforeToggle + 5 }
            audio.muted = true; SystemClock.sleep(100)
            val stopped = sent.get(); SystemClock.sleep(120)
            assertEquals("Mute stops microphone capture", stopped, sent.get())
            instrumentation.sendStatus(0, Bundle().apply { putString("nativeAudioResult", "codec=Opus/48000,captured=${sent.get()},nonzero_playout=${nonzero.get()}") })
        } finally { audio.close(); route.close() }
    }

    private fun await(description: String, ready: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 20_000
        while (!ready()) { check(SystemClock.elapsedRealtime() < deadline) { "Timed out: $description" }; SystemClock.sleep(40) }
    }
}
