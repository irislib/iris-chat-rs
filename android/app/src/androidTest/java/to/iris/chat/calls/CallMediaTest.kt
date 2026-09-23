package to.iris.chat.calls

import android.Manifest
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.SystemClock
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.CallSnapshot
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class CallMediaTest {
    @get:Rule val compose = createComposeRule()

    @Test fun incomingVideoCanBeAnsweredWithoutCameraPermission() {
        val actions = mutableListOf<AppAction>()
        val videoPermissions = mutableListOf<Boolean>()
        compose.setContent {
            IrisChatTheme {
                CallSurface(call("incoming"), true, true, null, null, null, false, 0,
                    permissions = { video, next -> videoPermissions += video; next() },
                    onAction = { actions += it }, onSpeaker = {}, onDismiss = {})
            }
        }
        compose.onNodeWithText("Incoming video call").assertExists()
        screenshot("incoming-video-call.png")
        compose.onNodeWithText("Answer with voice").performClick()
        assertEquals(listOf(false), videoPermissions)
        assertEquals(listOf(AppAction.AnswerCallWithVoice("test-call")), actions)
    }

    @Test fun connectedCallControlsDispatchMuteCameraAndHangup() {
        val actions = mutableListOf<AppAction>()
        compose.setContent {
            IrisChatTheme {
                CallSurface(call("connected"), true, true, null, null, null, true, 80,
                    permissions = { _, next -> next() }, onAction = { actions += it }, onSpeaker = {}, onDismiss = {})
            }
        }
        screenshot("connected-video-call.png")
        compose.onNodeWithContentDescription("Mute").performClick()
        compose.onNodeWithContentDescription("Camera").performClick()
        compose.onNodeWithContentDescription("End call").performClick()
        assertEquals(listOf(AppAction.SetCallMuted(true), AppAction.SetCallVideoEnabled(false), AppAction.EndCall("test-call")), actions)
    }

    @Test fun nativeMicrophoneFramesPlayLocallyAndStopOnMuteAndClose() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        compose.setContent { androidx.compose.material3.Text("Audio call test") }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val received = CountDownLatch(12)
        val count = AtomicInteger()
        val error = AtomicReference<String?>(null)
        lateinit var audio: CallAudio
        audio = CallAudio(context, scope, send = { bytes ->
            if (bytes.size != 640) error.set("Unexpected audio frame")
            count.incrementAndGet()
            audio.receive(bytes)
            received.countDown()
        }, failed = { error.set("Native audio failed") })
        try {
            audio.start(false)
            assertTrue("Microphone must produce PCM frames", received.await(10, TimeUnit.SECONDS))
            audio.muted = true
            // A read already in progress can complete; wait for several frame periods.
            Thread.sleep(120)
            val mutedCount = count.get()
            Thread.sleep(200)
            assertEquals("Muted microphone must not transmit", mutedCount, count.get())
            audio.close()
            Thread.sleep(120)
            assertEquals("Closed microphone must not transmit", mutedCount, count.get())
            assertEquals(null, error.get())
        } finally { audio.close(); scope.cancel() }
    }

    @Test fun nativeCameraProducesBoundedInteroperableJpeg() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.CAMERA)
        compose.setContent { androidx.compose.material3.Text("Video call test") }
        val ready = CountDownLatch(1)
        val frame = AtomicReference<ByteArray?>(null)
        val failed = AtomicInteger()
        val camera = CallCamera(context, send = { frame.set(it); ready.countDown() }, preview = {}, failed = { failed.incrementAndGet(); ready.countDown() })
        try {
            camera.start()
            assertTrue("Camera must produce a JPEG frame", ready.await(15, TimeUnit.SECONDS))
            assertEquals(0, failed.get())
            val bytes = frame.get()
            assertNotNull(bytes)
            assertTrue(bytes!!.size in 1..65_536)
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
            assertNotNull(bitmap)
            assertTrue(bitmap.width <= 320 && bitmap.height <= 240)
        } finally { camera.close() }
    }

    private fun call(phase: String) = CallSnapshot("test-call", "test-chat", "Alex", phase,
        video = true, videoCapable = true, muted = false, remoteVideo = true, remoteMuted = false,
        startedAtSecs = 0u, connectedAtSecs = if (phase == "connected") 0u else null, endReason = null)

    private fun screenshot(name: String) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var bitmap: Bitmap
        // Compose idleness does not include the platform dialog's opening animation.
        // Capture only once the call background has reached its final displayed color.
        while (true) {
            bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
            val center = bitmap.getPixel(bitmap.width / 2, bitmap.height / 2)
            if (Color.red(center) < 35 && Color.green(center) < 45 && Color.blue(center) < 45) break
            bitmap.recycle()
            check(SystemClock.elapsedRealtime() < deadline) { "Call screen did not finish appearing" }
            SystemClock.sleep(50)
        }
        val directory = checkNotNull(instrumentation.targetContext.getExternalFilesDir("screenshots"))
        directory.mkdirs()
        File(directory, name).outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }
}
