package to.iris.chat.calls

import android.graphics.Bitmap
import android.graphics.Color
import android.os.SystemClock
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
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

    @Test fun videoCallQualityCanChangeProfileAndBitrateDuringCall() {
        val actions = mutableListOf<AppAction>()
        compose.setContent { IrisChatTheme {
            CallSurface(call("connected"), true, true, null, null, null, false, 80,
                permissions = { _, next -> next() }, onAction = { actions += it }, onSpeaker = {}, onDismiss = {},
                quality = "high", customMaxBitrateBps = 750_000u)
        } }
        compose.onNodeWithContentDescription("Quality").performClick()
        compose.onNodeWithText("Custom").performClick()
        compose.onNodeWithTag("callQualityMaximum").performSemanticsAction(SemanticsActions.SetProgress) { it(0.6f) }
        compose.onNodeWithText("Maximum: 0.6 Mbps").assertExists()
        screenshot("in-call-quality.png", waitForCallBackground = false)
        compose.onNodeWithText("Save").performClick()
        assertEquals(AppAction.SetCallQuality("custom", 600_000u), actions.single())
        compose.onNodeWithContentDescription("Quality").performClick()
        compose.onNodeWithText("Use less data").performClick()
        compose.onNodeWithText("Save").performClick()
        assertEquals(AppAction.SetCallQuality("data", 750_000u), actions.last())
    }

    @Test fun voiceCallDoesNotShowVideoQualityControl() {
        compose.setContent { IrisChatTheme {
            CallSurface(call("connected").copy(video = false, videoCapable = false, remoteVideo = false),
                true, true, null, null, null, false, 80, permissions = { _, next -> next() },
                onAction = {}, onSpeaker = {}, onDismiss = {})
        } }
        compose.onNodeWithContentDescription("Quality").assertDoesNotExist()
    }

    private fun call(phase: String) = CallSnapshot(callId = "test-call", chatId = "test-chat", peerName = "Alex", phase = phase,
        video = true, videoCapable = true, muted = false, remoteVideo = true, remoteMuted = false,
        startedAtSecs = 0u, connectedAtSecs = if (phase == "connected") 0u else null, endReason = null,
        outgoing = phase != "incoming", targetBitrateBps = 2_000_000u, keyFrameGeneration = 0u, mediaConnected = phase == "connected", maxBitrateBps = 2_000_000u)

    private fun screenshot(name: String, waitForCallBackground: Boolean = true) {
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        if (!waitForCallBackground) SystemClock.sleep(250)
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var bitmap: Bitmap
        // Compose idleness does not include the platform dialog's opening animation.
        // Capture only once the call background has reached its final displayed color.
        while (true) {
            bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
            val center = bitmap.getPixel(bitmap.width / 2, bitmap.height / 2)
            if (!waitForCallBackground || Color.red(center) < 35 && Color.green(center) < 45 && Color.blue(center) < 45) break
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
