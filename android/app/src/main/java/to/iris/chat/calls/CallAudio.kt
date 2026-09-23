package to.iris.chat.calls

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** Wire format: 20 ms of signed little-endian PCM, mono, 16 kHz. */
internal class CallAudio(
    context: Context,
    private val scope: CoroutineScope,
    private val send: (ByteArray) -> Unit,
    private val failed: () -> Unit,
) : AutoCloseable {
    private val manager = context.getSystemService(AudioManager::class.java)
    private val attributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        .setAudioAttributes(attributes)
        .setOnAudioFocusChangeListener { change ->
            if (change == AudioManager.AUDIOFOCUS_LOSS) failed()
        }.build()
    private val playback = Channel<ByteArray>(8, BufferOverflow.DROP_OLDEST)
    private var recorder: AudioRecord? = null
    private var player: AudioTrack? = null
    private var echo: AcousticEchoCanceler? = null
    private var noise: NoiseSuppressor? = null
    private var captureJob: Job? = null
    private var playbackJob: Job? = null
    private var oldMode = AudioManager.MODE_NORMAL
    private var oldSpeaker = false
    @Volatile var muted = false
    @Volatile private var closed = false

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    fun start(speaker: Boolean) {
        check(!closed)
        oldMode = manager.mode
        oldSpeaker = manager.isSpeakerphoneOn
        check(manager.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
        manager.mode = AudioManager.MODE_IN_COMMUNICATION
        manager.isSpeakerphoneOn = speaker
        val inputSize = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val input = AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
            AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(inputSize, FRAME_BYTES * 4))
        recorder = input
        check(input.state == AudioRecord.STATE_INITIALIZED)
        if (AcousticEchoCanceler.isAvailable()) {
            echo = AcousticEchoCanceler.create(input.audioSessionId)?.apply { enabled = true }
        }
        if (NoiseSuppressor.isAvailable()) {
            noise = NoiseSuppressor.create(input.audioSessionId)?.apply { enabled = true }
        }
        val outputSize = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val output = AudioTrack.Builder().setAudioAttributes(attributes)
            .setAudioFormat(AudioFormat.Builder().setSampleRate(RATE)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(maxOf(outputSize, FRAME_BYTES * 4))
            .setTransferMode(AudioTrack.MODE_STREAM).build()
        player = output
        check(output.state == AudioTrack.STATE_INITIALIZED)
        output.play()
        input.startRecording()
        captureJob = scope.launch(Dispatchers.IO) {
            try {
                val frame = ByteArray(FRAME_BYTES)
                var offset = 0
                while (isActive && !closed) {
                    val count = input.read(frame, offset, frame.size - offset, AudioRecord.READ_BLOCKING)
                    check(count > 0)
                    offset += count
                    if (offset == frame.size) {
                        if (!muted) send(frame.copyOf())
                        offset = 0
                    }
                }
            } catch (_: Exception) {
                if (!closed) failed()
            }
        }
        playbackJob = scope.launch(Dispatchers.IO) {
            try {
                for (frame in playback) {
                    var offset = 0
                    while (offset < frame.size && isActive && !closed) {
                        val count = output.write(frame, offset, frame.size - offset, AudioTrack.WRITE_BLOCKING)
                        check(count > 0)
                        offset += count
                    }
                }
            } catch (_: Exception) {
                if (!closed) failed()
            }
        }
    }

    fun receive(data: ByteArray) {
        if (data.size == FRAME_BYTES && !closed) playback.trySend(data)
    }

    @Suppress("DEPRECATION")
    fun setSpeaker(enabled: Boolean) { manager.isSpeakerphoneOn = enabled }

    @Suppress("DEPRECATION")
    override fun close() {
        if (closed) return
        closed = true
        captureJob?.cancel()
        playbackJob?.cancel()
        playback.close()
        runCatching { recorder?.stop() }
        runCatching { player?.stop() }
        echo?.release()
        noise?.release()
        recorder?.release()
        player?.release()
        manager.abandonAudioFocusRequest(focus)
        manager.isSpeakerphoneOn = oldSpeaker
        manager.mode = oldMode
    }

    companion object {
        const val RATE = 16_000
        const val FRAME_BYTES = 640
    }
}
