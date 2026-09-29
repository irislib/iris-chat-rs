package to.iris.chat.calls

import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.MediaPlayer
import android.os.SystemClock
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.CallSnapshot

/** Exercises the packaged WAVs and asynchronous platform player, without opening an account. */
@RunWith(AndroidJUnit4::class)
class CallTonesTest {
    @get:Rule val compose = createComposeRule()
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    @Test fun outgoingAndRingingLoopOnCallAudioWithoutMicrophoneAndStopOnAnswer() {
        compose.setContent {}
        compose.waitForIdle()
        val audioManager = context.getSystemService(AudioManager::class.java)
        val tones = CallTones(context)
        val route = CallAudioRoute(context) { error("Lost test call audio focus") }
        try {
            onMain { route.start(speaker = true); tones.update(call("outgoing")) }
            val connecting = awaitPlaying(tones)
            onMain {
                assertTrue(connecting.isLooping)
                assertEquals(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, connecting.routedDevice?.type)
                assertTrue(audioManager.activePlaybackConfigurations.any { it.audioAttributes.usage == AudioAttributes.USAGE_VOICE_COMMUNICATION })
                tones.update(call("outgoing"))
                assertSame("Unchanged snapshots must not restart the tone", connecting, player(tones))
                tones.update(call("ringing"))
            }
            val ringing = awaitPlaying(tones)
            onMain {
                assertNotSame(connecting, ringing)
                assertTrue(ringing.isLooping)
                assertEquals(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, ringing.routedDevice?.type)
                assertTrue(audioManager.activePlaybackConfigurations.any { it.audioAttributes.usage == AudioAttributes.USAGE_VOICE_COMMUNICATION })
                assertTrue("Ringback must not capture the microphone", audioManager.activeRecordingConfigurations.isEmpty())
                tones.update(call("connected"))
                assertNull(player(tones))
            }
        } finally { onMain { tones.stop(); route.close() } }
    }

    @Test fun endingOrLosingFocusDuringPreparationCannotStartALateTone() {
        val tones = CallTones(context)
        try {
            for (phase in listOf("outgoing", "ringing")) {
                onMain { tones.update(call(phase)); tones.stop() }
                SystemClock.sleep(150)
                onMain { assertNull(player(tones)) }
                onMain { tones.update(call(phase)) }
                awaitPlaying(tones)
                onMain { tones.update(call("ended")); assertNull(player(tones)) }
            }
            onMain {
                tones.update(call("incoming").copy(outgoing = false))
                assertNull("Incoming calls must use the system ringtone", player(tones))
                tones.update(null)
                assertNull(player(tones))
            }
        } finally { onMain { tones.stop() } }
    }

    private fun awaitPlaying(tones: CallTones): MediaPlayer {
        var playing: MediaPlayer? = null
        val deadline = SystemClock.elapsedRealtime() + 5_000
        while (playing == null && SystemClock.elapsedRealtime() < deadline) {
            onMain { playing = player(tones)?.takeIf { it.isPlaying && it.currentPosition > 0 && it.routedDevice != null } }
            if (playing == null) SystemClock.sleep(20)
        }
        return checkNotNull(playing) { "Packaged call tone did not begin playback" }
    }

    // Inspect the real player rather than adding a second implementation or a test-only runtime API.
    private fun player(tones: CallTones): MediaPlayer? =
        CallTones::class.java.getDeclaredField("player").apply { isAccessible = true }.get(tones) as MediaPlayer?

    private fun onMain(action: () -> Unit) = instrumentation.runOnMainSync(action)

    private fun call(phase: String) = CallSnapshot(callId = "ringback-test", chatId = "test-chat", peerName = "Alex", phase = phase,
        video = false, videoCapable = false, muted = false, remoteVideo = false, remoteMuted = false,
        startedAtSecs = 0u, connectedAtSecs = null, endReason = null,
        outgoing = true, targetBitrateBps = 2_000_000u, keyFrameGeneration = 0u, mediaConnected = false, maxBitrateBps = 2_000_000u)
}
