package to.iris.chat.calls

import android.media.MediaCodec
import android.media.MediaFormat
import android.opengl.GLES20
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import org.webrtc.EglBase
import org.webrtc.GlRectDrawer
import org.webrtc.VideoFrame
import org.webrtc.VideoFrameDrawer

internal class CallVideoEncoder(
    private val shared: EglBase.Context,
    private val send: (ByteArray, ULong, Boolean) -> Unit,
    private val failed: () -> Unit,
) : AutoCloseable {
    private val thread = HandlerThread("Iris-video-encode").apply { start() }
    private val handler = Handler(thread.looper)
    private val closed = AtomicBoolean(false)
    private val framePending = AtomicBoolean(false)
    private val generation = AtomicLong()
    private val privacyLock = Any()
    private var encoderGeneration = -1L
    private var codec: MediaCodec? = null
    private var surface: Surface? = null
    private var egl: EglBase? = null
    private var drawer: GlRectDrawer? = null
    private var frames: VideoFrameDrawer? = null
    private var dimensions: Pair<Int, Int>? = null
    private var requestedSize: Pair<Int, Int>? = null
    private var headers = emptyList<ByteArray>()
    private var lastFrameNs = 0L
    private var lastKeyRequestNs = 0L
    private var needsOutputKey = true
    @Volatile private var profile = "auto"
    @Volatile private var targetBitrate = 2_000_000
    @Volatile private var enabled = true

    fun update(enabled: Boolean, profile: String, bitrate: Int, requestKey: Boolean) {
        val toggled = synchronized(privacyLock) {
            val changed = this.enabled != enabled
            if (changed) generation.incrementAndGet()
            this.enabled = enabled
            changed
        }
        this.profile = profile
        this.targetBitrate = bitrate.coerceIn(100_000, 10_000_000)
        handler.post {
            if (toggled && !enabled) releaseEncoder()
            if (!closed.get()) runCatching {
                if (requestKey) needsOutputKey = true
                codec?.setParameters(Bundle().apply {
                    putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, targetBitrate)
                    if (requestKey) putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
                })
            }.onFailure { failed() }
        }
    }

    fun frame(frame: VideoFrame) {
        if (closed.get() || !enabled || !framePending.compareAndSet(false, true)) return
        val ticket = generation.get()
        frame.retain()
        handler.post {
            try {
                if (closed.get() || !enabled || ticket != generation.get()) return@post
                val fps = if (targetBitrate < 200_000) 10 else if (profile == "data" || targetBitrate < 500_000) 15 else 30
                if (frame.timestampNs - lastFrameNs < 950_000_000L / fps) return@post
                lastFrameNs = frame.timestampNs
                val longEdge = when {
                    targetBitrate < 200_000 -> 320
                    targetBitrate < 500_000 || profile == "data" -> 640
                    targetBitrate < 1_000_000 -> 960
                    profile == "high" && targetBitrate >= 3_000_000 -> 1920
                    else -> 1280
                }
                val scale = minOf(1.0, longEdge.toDouble() / maxOf(frame.rotatedWidth, frame.rotatedHeight),
                    kotlin.math.sqrt((1920.0 * 1080) / (frame.rotatedWidth.toDouble() * frame.rotatedHeight)))
                var width = (frame.rotatedWidth * scale).toInt() / 2 * 2
                var height = (frame.rotatedHeight * scale).toInt() / 2 * 2
                if (codec == null || requestedSize != (width to height)) {
                    releaseEncoder()
                    val requested = width to height
                    while (!initialize(width, height, fps, ticket)) {
                        check(maxOf(width, height) > 640) { "No compatible H.264 encoder" }
                        width = (width * 0.75).toInt() / 2 * 2
                        height = (height * 0.75).toInt() / 2 * 2
                    }
                    requestedSize = requested
                }
                if (frame.timestampNs - lastKeyRequestNs >= 1_000_000_000L) {
                    lastKeyRequestNs = frame.timestampNs
                    codec?.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
                }
                val active = checkNotNull(egl)
                val size = checkNotNull(dimensions)
                active.makeCurrent()
                GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
                checkNotNull(frames).drawFrame(frame, checkNotNull(drawer), null, 0, 0, size.first, size.second)
                active.swapBuffers(frame.timestampNs)
            } catch (_: Exception) { if (!closed.get()) failed() }
            finally { frame.release(); framePending.set(false) }
        }
    }

    private fun initialize(width: Int, height: Int, fps: Int, ticket: Long): Boolean {
        for ((name, format) in CallAvc.encoderFormats(width, height, fps, targetBitrate)) {
            try {
                val candidate = MediaCodec.createByCodecName(name)
                codec = candidate
                encoderGeneration = ticket
                candidate.setCallback(callback, handler)
                candidate.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                surface = candidate.createInputSurface()
                egl = EglBase.create(shared, EglBase.CONFIG_RECORDABLE).apply { createSurface(checkNotNull(surface)); makeCurrent() }
                drawer = GlRectDrawer(); frames = VideoFrameDrawer()
                dimensions = width to height
                candidate.start()
                android.util.Log.i("IrisCallCodec", "AVC encoder $name: ${width}x$height at $fps fps, $targetBitrate bps")
                return true
            } catch (_: Exception) { releaseEncoder() }
        }
        return false
    }

    private val callback = object : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) = Unit
        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {
            if (codec !== this@CallVideoEncoder.codec) return
            runCatching {
                headers = listOfNotNull(format.getByteBuffer("csd-0"), format.getByteBuffer("csd-1"))
                    .flatMap { buffer -> ByteArray(buffer.remaining()).also { buffer.duplicate().get(it) }.let(CallAvc::nals) }
                    .filter { (it[0].toInt() and 31) in 7..8 }
            }.onFailure { failed() }
        }
        override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) {
            if (codec === this@CallVideoEncoder.codec && !closed.get()) { releaseEncoder(); failed() }
        }
        override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
            if (codec !== this@CallVideoEncoder.codec) return
            try {
                if (info.size <= 0 || !enabled || closed.get() || encoderGeneration != generation.get()) return
                if (info.size > CallAvc.MAX_FRAME) {
                    needsOutputKey = true
                    codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
                    return
                }
                val buffer = checkNotNull(codec.getOutputBuffer(index)).duplicate()
                buffer.position(info.offset); buffer.limit(info.offset + info.size)
                val bytes = ByteArray(info.size).also { buffer.get(it) }
                val nals = CallAvc.nals(bytes)
                if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                    headers = nals.filter { (it[0].toInt() and 31) in 7..8 }
                    return
                }
                val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                if (needsOutputKey && !key) return
                val frame = if (key) {
                    val types = nals.map { it[0].toInt() and 31 }.toSet()
                    CallAvc.annexB(headers.filter { (it[0].toInt() and 31) !in types } + nals)
                } else bytes
                if (frame.size <= CallAvc.MAX_FRAME) {
                    synchronized(privacyLock) {
                        if (enabled && !closed.get() && encoderGeneration == generation.get()) {
                            needsOutputKey = false
                            send(frame, info.presentationTimeUs.toULong(), key)
                        }
                    }
                } else {
                    needsOutputKey = true
                    codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
                }
            } catch (_: Exception) { if (!closed.get()) failed() }
            finally { codec.releaseOutputBuffer(index, false) }
        }
    }

    private fun releaseEncoder() {
        runCatching { egl?.makeCurrent(); frames?.release(); drawer?.release() }
        frames = null; drawer = null
        runCatching { egl?.release() }; egl = null
        runCatching { codec?.stop() }; runCatching { codec?.release() }; codec = null
        surface?.release(); surface = null; encoderGeneration = -1L; dimensions = null; requestedSize = null; headers = emptyList(); needsOutputKey = true
    }

    fun awaitClosed() { thread.join() }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        handler.post { releaseEncoder(); thread.quitSafely() }
    }
}
