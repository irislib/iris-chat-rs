package to.iris.chat.audio

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.PlaybackParams
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal class VoiceMessagePlayback(
    context: Context,
    private val scope: CoroutineScope,
    private val download: suspend () -> ByteArray?,
) : AutoCloseable {
    private val appContext = context.applicationContext
    private val audio = appContext.getSystemService(AudioManager::class.java)
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        .setAudioAttributes(attributes).setOnAudioFocusChangeListener { change ->
            if (change != AudioManager.AUDIOFOCUS_GAIN) pause()
        }.build()
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) { pause() }
    }
    private var player: MediaPlayer? = null
    private var file: File? = null
    private var load: Job? = null
    private var ticker: Job? = null
    private var prepared = false
    private var closed = false
    var playing by mutableStateOf(false); private set
    var loading by mutableStateOf(false); private set
    var elapsed by mutableStateOf(0f); private set
    var duration by mutableStateOf(0f); private set
    var rate by mutableStateOf(1f); private set
    var peaks by mutableStateOf(emptyList<Float>()); private set
    var error by mutableStateOf<String?>(null); private set

    init {
        ContextCompat.registerReceiver(appContext, noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    fun toggle() { if (playing || loading) pause() else play() }

    fun play() {
        if (closed) return
        if (callActive) { error = "Finish your call to play audio."; return }
        if (active !== this) active?.pause()
        active = this
        error = null
        if (prepared) { start(); return }
        if (loading) return
        loading = true
        load = scope.launch {
            val candidate = File(appContext.cacheDir, "iris-audio-${UUID.randomUUID()}")
            try {
                val bytes = download()?.takeIf { it.isNotEmpty() } ?: error("Audio unavailable")
                ensureActive()
                withContext(Dispatchers.IO) { candidate.writeBytes(bytes) }
                val waveform = AudioWaveform.decode(candidate)
                ensureActive()
                peaks = waveform
                releasePlayer()
                file = candidate
                val native = MediaPlayer()
                player = native
                native.setAudioAttributes(attributes)
                native.setDataSource(candidate.path)
                native.setOnPreparedListener {
                    if (player !== it || !loading) return@setOnPreparedListener
                    duration = it.duration / 1000f
                    prepared = true
                    start()
                }
                native.setOnCompletionListener {
                    if (player === it) { pause(); elapsed = duration }
                }
                native.setOnErrorListener { media, _, _ ->
                    if (player === media) fail()
                    true
                }
                native.prepareAsync()
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) { fail() }
            finally { if (file !== candidate) candidate.delete() }
        }
    }

    private fun start() {
        val native = player ?: return
        if (callActive || closed) { pause(); return }
        if (audio.requestAudioFocus(focus) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) { fail(); return }
        try {
            if (elapsed >= duration - 0.05f) seek(0f)
            native.playbackParams = PlaybackParams().setSpeed(rate).setPitch(1f)
            native.start()
            loading = false
            playing = true
            ticker?.cancel()
            ticker = scope.launch {
                while (playing) { elapsed = native.currentPosition / 1000f; delay(100) }
            }
        } catch (_: Exception) { fail() }
    }

    fun pause() {
        load?.cancel(); load = null
        ticker?.cancel(); ticker = null
        if (prepared) runCatching { player?.pause() } else releasePlayer()
        playing = false
        loading = false
        audio.abandonAudioFocusRequest(focus)
    }

    fun seek(seconds: Float) {
        if (!prepared || !seconds.isFinite()) return
        elapsed = seconds.coerceIn(0f, duration)
        player?.seekTo((elapsed * 1000).toLong(), MediaPlayer.SEEK_CLOSEST)
    }

    fun cycleRate() {
        rate = when (rate) { 1f -> 1.5f; 1.5f -> 2f; 2f -> 0.5f; else -> 1f }
        // setPlaybackParams starts paused MediaPlayers, so only apply while playing.
        if (playing) runCatching { player?.playbackParams = PlaybackParams().setSpeed(rate).setPitch(1f) }.onFailure { fail() }
    }

    private fun fail() { pause(); releasePlayer(); error = "Couldn't play audio. Try again." }
    private fun releasePlayer() { player?.release(); player = null; prepared = false; file?.delete(); file = null }

    override fun close() {
        if (closed) return
        closed = true
        pause(); releasePlayer()
        appContext.unregisterReceiver(noisy)
        if (active === this) active = null
    }

    companion object {
        private var active: VoiceMessagePlayback? = null
        @Volatile private var callActive = false
        fun setCallActive(value: Boolean) {
            callActive = value
            if (value) {
                if (Looper.myLooper() == Looper.getMainLooper()) active?.pause()
                else Handler(Looper.getMainLooper()).post { active?.pause() }
            }
        }
    }
}
