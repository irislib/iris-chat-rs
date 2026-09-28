package to.iris.chat.ui.screens

import android.graphics.Bitmap
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.audio.AudioWaveform
import to.iris.chat.audio.VoiceMessagePlayback
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.ui.theme.IrisChatTheme
import to.iris.chat.ui.theme.IrisTheme

class ChatAudioMessageTest {
    @get:Rule val compose = createComposeRule()
    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private fun bytes() = instrumentation.context.assets.open("audio/voice-message.m4a").use { it.readBytes() }

    @Test fun nativePlaybackWaveformSeekingCallsAndCancellation() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val data = bytes()
        lateinit var first: VoiceMessagePlayback
        lateinit var second: VoiceMessagePlayback
        var downloads = 0
        compose.runOnUiThread {
            VoiceMessagePlayback.setCallActive(false)
            first = VoiceMessagePlayback(instrumentation.targetContext, scope) { downloads++; data }
            second = VoiceMessagePlayback(instrumentation.targetContext, scope) { data }
            assertEquals(0, downloads)
            first.play()
        }
        try {
            compose.waitUntil(10_000) { first.playing && first.elapsed > 0.1f }
            assertEquals(6f, first.duration, 0.2f)
            assertEquals(47, first.peaks.size)
            assertTrue(first.peaks.subList(18, 22).all { it < 0.02f })
            assertTrue(first.peaks.take(14).any { it > 0.5f })
            compose.runOnUiThread {
                first.pause(); first.seek(3f)
                assertEquals(3f, first.elapsed, 0.1f)
                for (rate in listOf(1.5f, 2f, 0.5f, 1f)) { first.cycleRate(); assertEquals(rate, first.rate); assertFalse(first.playing) }
                first.play()
                assertEquals(1, downloads)
                second.play()
                assertFalse(first.playing)
            }
            compose.waitUntil(10_000) { second.playing }
            compose.runOnUiThread {
                VoiceMessagePlayback.setCallActive(true)
                assertFalse(second.playing)
                first.play(); assertNotNull(first.error)
                VoiceMessagePlayback.setCallActive(false)
                second.play(); second.seek(5.8f)
            }
            compose.waitUntil(5_000) { !second.playing }
            compose.runOnUiThread { second.play() }
            compose.waitUntil(5_000) { second.playing && second.elapsed < 1f }
            val pending = CompletableDeferred<ByteArray?>()
            lateinit var delayed: VoiceMessagePlayback
            compose.runOnUiThread {
                delayed = VoiceMessagePlayback(instrumentation.targetContext, scope) { pending.await() }
                delayed.play(); delayed.pause(); pending.complete(data)
            }
            compose.waitForIdle()
            assertFalse(delayed.playing); assertFalse(delayed.loading); assertNull(delayed.error)
            compose.runOnUiThread { delayed.close() }
        } finally {
            compose.runOnUiThread { first.close(); second.close(); VoiceMessagePlayback.setCallActive(false); scope.cancel() }
        }
    }

    @Test fun incomingAndOutgoingAttachmentsHaveSeekableInlineWaveforms() {
        val data = bytes()
        val attachment = MessageAttachmentSnapshot("fixture", "Voice message.M4A", "Voice%20message.M4A", "htree://fixture/Voice%20message.M4A", false, false, false)
        var downloads = 0
        compose.setContent {
            IrisChatTheme(darkTheme = true) {
                Surface(color = androidx.compose.material3.MaterialTheme.colorScheme.background) {
                    Column(Modifier.fillMaxSize().padding(20.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
                        for (mine in listOf(false, true)) {
                            Surface(color = if (mine) IrisTheme.palette.bubbleMine else IrisTheme.palette.bubbleTheirs,
                                shape = RoundedCornerShape(18.dp), modifier = Modifier.align(if (mine) Alignment.End else Alignment.Start)) {
                                Box(Modifier.padding(12.dp)) {
                                    AttachmentChip(attachment, mine, { downloads++; data }, { _, _ -> fail("Audio opened externally") }, {})
                                }
                            }
                        }
                    }
                }
            }
        }
        assertEquals(0, downloads)
        compose.onAllNodesWithContentDescription("Play audio")[0].performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Pause audio").fetchSemanticsNodes().size == 1 }
        compose.onNodeWithContentDescription("Pause audio").performClick()
        val progress = compose.onAllNodesWithTag("chatAudioProgress")[0]
        progress.performTouchInput { click(Offset(width * 0.75f, height / 2f)) }
        assertEquals(4.5f, progress.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current, 0.2f)
        progress.performSemanticsAction(SemanticsActions.SetProgress) { it(3f) }
        assertEquals(3f, progress.fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current, 0.1f)
        compose.onAllNodesWithTag("chatAudioSpeedButton")[0].performClick()
        compose.onNodeWithText("1.5×").assertExists()
        compose.onAllNodesWithContentDescription("Play audio")[1].performClick()
        compose.waitUntil(10_000) { compose.onAllNodesWithContentDescription("Pause audio").fetchSemanticsNodes().size == 1 }
        compose.onNodeWithContentDescription("Pause audio").performClick()
        compose.waitForIdle()
        File(instrumentation.targetContext.getExternalFilesDir(null), "android-inline-voice-messages.png").outputStream().use {
            instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        assertEquals(2, downloads)
    }
}
