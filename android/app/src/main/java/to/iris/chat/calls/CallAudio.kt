package to.iris.chat.calls

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Process
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import to.iris.chat.rust.CallAudioCodec

/** 48 kHz capture; the shared codec supplies Opus, jitter buffering, FEC and concealment. */
internal class CallAudio(
    private val context: Context,
    private val send: (ByteArray, ULong) -> Unit,
    private val failed: () -> Unit,
    private val played: (ShortArray) -> Unit = {},
) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    private val codec = CallAudioCodec()
    private var recorder: AudioRecord? = null
    private var player: AudioTrack? = null
    private var echo: AcousticEchoCanceler? = null
    private var noise: NoiseSuppressor? = null
    private var capture: Thread? = null
    private var playback: Thread? = null
    private val captureLock = Any()
    private val generation = AtomicLong()
    @Volatile private var microphoneMuted = false
    var muted: Boolean
        get() = microphoneMuted
        set(value) = synchronized(captureLock) {
            if (microphoneMuted != value) {
                microphoneMuted = value
                generation.incrementAndGet()
                if (value) runCatching { recorder?.stop() }
            }
        }

    @SuppressLint("MissingPermission")
    fun start(muted: Boolean) {
        this.muted = muted
        val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        check(minimum > 0)
        val input = AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
            AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minimum, SAMPLES * 8))
        recorder = input
        check(input.state == AudioRecord.STATE_INITIALIZED)
        if (AcousticEchoCanceler.isAvailable()) echo = AcousticEchoCanceler.create(input.audioSessionId)?.apply { enabled = true }
        if (NoiseSuppressor.isAvailable()) noise = NoiseSuppressor.create(input.audioSessionId)?.apply { enabled = true }
        val outputMinimum = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val output = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            .setAudioFormat(AudioFormat.Builder().setSampleRate(RATE).setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(maxOf(outputMinimum, SAMPLES * 8)).setTransferMode(AudioTrack.MODE_STREAM).build()
        player = output
        check(output.state == AudioTrack.STATE_INITIALIZED)
        output.play()
        capture = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
            try {
                val samples = ShortArray(SAMPLES)
                var offset = 0
                var bufferGeneration = -1L
                while (!closed.get()) {
                    val ticket = synchronized(captureLock) {
                        if (this.muted || closed.get()) null else {
                            if (input.recordingState != AudioRecord.RECORDSTATE_RECORDING) input.startRecording()
                            generation.get()
                        }
                    }
                    if (ticket == null) { offset = 0; Thread.sleep(10); continue }
                    if (bufferGeneration != ticket) { offset = 0; bufferGeneration = ticket }
                    val count = input.read(samples, offset, samples.size - offset, AudioRecord.READ_BLOCKING)
                    if (closed.get()) break
                    if (ticket != generation.get() || this.muted) { offset = 0; continue }
                    check(count > 0)
                    offset += count
                    if (offset == samples.size) {
                        synchronized(captureLock) {
                            if (!this.muted && ticket == generation.get() && !closed.get())
                                send(codec.encode(samples.toList()), (System.nanoTime() / 1_000).toULong())
                        }
                        offset = 0
                    }
                }
            } catch (_: Exception) { if (!closed.get()) failed() }
        }, "Iris-microphone").apply { start() }
        playback = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
            try {
                var due = System.nanoTime()
                while (!closed.get()) {
                    val remaining = due - System.nanoTime()
                    if (remaining > 0) java.util.concurrent.locks.LockSupport.parkNanos(remaining)
                    if (closed.get()) break
                    val samples = codec.playout().toShortArray()
                    var offset = 0
                    while (offset < samples.size && !closed.get()) {
                        val count = output.write(samples, offset, samples.size - offset, AudioTrack.WRITE_BLOCKING)
                        if (closed.get()) break
                        check(count > 0)
                        offset += count
                    }
                    if (!closed.get()) played(samples)
                    due = maxOf(due + 20_000_000, System.nanoTime())
                }
            } catch (_: Exception) { if (!closed.get()) failed() }
        }, "Iris-speaker").apply { start() }
    }

    fun receive(sequence: UInt, packet: ByteArray) {
        if (!closed.get() && packet.size in 1..1275) codec.queue(sequence, packet)
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        synchronized(captureLock) { runCatching { recorder?.stop() } }
        runCatching { player?.pause(); player?.flush() }
        capture?.interrupt(); playback?.interrupt()
        // Never free native audio/codec objects until both owners have exited.
        Thread({
            capture?.join(); playback?.join()
            echo?.release(); noise?.release()
            recorder?.release(); player?.release(); codec.close()
        }, "Iris-audio-close").start()
    }

    companion object { const val RATE = 48_000; const val SAMPLES = 960 }
}
