package to.iris.chat.calls

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import to.iris.chat.rust.CallSnapshot

/** Output only, using the self-managed call's audio focus and route. */
internal class CallTones(private val context: Context) {
    private var player: MediaPlayer? = null
    private var key: String? = null

    fun update(call: CallSnapshot?) {
        val tone = when {
            call?.outgoing != true -> null
            call.phase == "outgoing" -> "connecting"
            call.phase == "ringing" -> "ringing"
            else -> null
        }
        val next = tone?.let { "${call?.callId}:$it" }
        if (next == key) return
        stop()
        key = next
        if (tone == null) return
        try {
            val playing = MediaPlayer()
            player = playing
            playing.setAudioAttributes(AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build())
            context.assets.openFd("call-$tone.wav").use {
                playing.setDataSource(it.fileDescriptor, it.startOffset, it.length)
            }
            playing.isLooping = true
            playing.setOnPreparedListener { if (player === it) it.start() }
            playing.setOnErrorListener { _, _, _ -> stop(); true }
            playing.prepareAsync()
        } catch (_: Exception) { stop() }
    }

    fun stop() { player?.release(); player = null; key = null }
}
