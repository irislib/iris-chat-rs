package to.iris.chat.calls

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.webrtc.EglBase
import org.webrtc.SurfaceTextureHelper

internal class CallVideoDecoder(
    shared: EglBase.Context,
    private val output: CallVideoStream,
    private val requestKey: () -> Unit,
    private val decoded: () -> Unit,
) : AutoCloseable {
    private data class Packet(val data: ByteArray, val timestamp: ULong, val key: Boolean)
    private val thread = HandlerThread("Iris-video-decode").apply { start() }
    private val handler = Handler(thread.looper)
    private val closed = AtomicBoolean(false)
    private val pending = AtomicInteger()
    private val textures = SurfaceTextureHelper.create("Iris-decoded-video", shared)
    private val surface = Surface(textures.surfaceTexture)
    private var codec: MediaCodec? = null
    private var sps: ByteArray? = null
    private val input = ArrayDeque<Int>()
    private val packets = ArrayDeque<Packet>()
    private val reorder = mutableMapOf<UInt, Packet>()
    private var expected: UInt? = null
    private var gapSince = 0L
    private var needsKey = true
    private var requestedAt = 0L

    init { textures.startListening { frame -> if (!closed.get()) { output.onFrame(frame); decoded() } } }

    fun receive(sequence: UInt, bytes: ByteArray, timestamp: ULong, key: Boolean) {
        if (closed.get() || bytes.size !in 1..CallAvc.MAX_FRAME) return
        if (pending.incrementAndGet() > 8) { pending.decrementAndGet(); handler.post { loseReference() }; return }
        handler.post {
            try {
                if (closed.get()) return@post
                val next = expected ?: sequence.also { expected = it }
                if ((sequence - next).toInt() < 0) return@post
                reorder[sequence] = Packet(bytes, timestamp, key)
                drainOrdered()
            } finally { pending.decrementAndGet() }
        }
    }

    private fun drainOrdered() {
        while (true) {
            val next = expected ?: return
            val packet = reorder.remove(next) ?: break
            expected = next + 1u
            decode(packet)
        }
        if (reorder.isEmpty()) { gapSince = 0; return }
        val now = SystemClock.elapsedRealtime()
        if (gapSince == 0L) { gapSince = now; handler.postDelayed({ if (!closed.get()) drainOrdered() }, 50) }
        if (reorder.size > 3 || now - gapSince >= 50) {
            val next = checkNotNull(expected)
            val key = reorder.entries.filter { it.value.key }.minByOrNull { (it.key - next).toLong() }
            loseReference()
            val following = if (key == null) emptyMap() else reorder.filterKeys { (it - key.key).toInt() > 0 }
            reorder.clear(); gapSince = 0
            if (key != null) {
                expected = key.key + 1u; decode(key.value)
                reorder.putAll(following); drainOrdered()
            }
        }
    }

    private fun decode(packet: Packet) {
        try {
            if (needsKey && !packet.key) { askForKey(); return }
            if (packet.key) {
                val nals = CallAvc.nals(packet.data)
                val newSps = nals.first { (it[0].toInt() and 31) == 7 }
                val pps = nals.first { (it[0].toInt() and 31) == 8 }
                require(nals.any { (it[0].toInt() and 31) == 5 })
                val dimensions = CallAvc.dimensions(newSps)
                if (codec == null || sps?.contentEquals(newSps) != true) configure(newSps, pps, dimensions)
                needsKey = false
            }
            if (packets.size >= 4) { loseReference(); return }
            packets.addLast(packet)
            drain()
        } catch (error: Exception) { android.util.Log.w("IrisCallCodec", "AVC frame rejected", error); loseReference() }
    }

    private fun configure(sps: ByteArray, pps: ByteArray, size: Pair<Int, Int>) {
        releaseDecoder()
        textures.setTextureSize(size.first, size.second)
        val format = MediaFormat.createVideoFormat(CallAvc.MIME, size.first, size.second).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(CallAvc.annexB(listOf(sps))))
            setByteBuffer("csd-1", ByteBuffer.wrap(CallAvc.annexB(listOf(pps))))
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, CallAvc.MAX_FRAME)
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        }
        val decoder = MediaCodec.createDecoderByType(CallAvc.MIME)
        codec = decoder
        decoder.setCallback(callback, handler)
        decoder.configure(format, surface, null, 0)
        decoder.start()
        android.util.Log.i("IrisCallCodec", "AVC decoder ${decoder.name}: ${size.first}x${size.second}")
        this.sps = sps
    }

    private fun drain() {
        val active = codec ?: return
        while (input.isNotEmpty() && packets.isNotEmpty()) {
            val index = input.removeFirst()
            val packet = packets.removeFirst()
            val buffer = checkNotNull(active.getInputBuffer(index))
            check(buffer.capacity() >= packet.data.size)
            buffer.clear(); buffer.put(packet.data)
            active.queueInputBuffer(index, 0, packet.data.size, packet.timestamp.toLong(), 0)
        }
    }

    private val callback = object : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
            if (codec !== this@CallVideoDecoder.codec) return
            input.addLast(index)
            runCatching { drain() }.onFailure { loseReference() }
        }
        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) = Unit
        override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) {
            if (codec === this@CallVideoDecoder.codec) { releaseDecoder(); loseReference() }
        }
        override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
            if (codec === this@CallVideoDecoder.codec) codec.releaseOutputBuffer(index, !closed.get())
        }
    }

    private fun loseReference() { needsKey = true; packets.clear(); askForKey() }
    private fun askForKey() {
        val now = SystemClock.elapsedRealtime()
        if (now - requestedAt > 1_000 && !closed.get()) { requestedAt = now; requestKey() }
    }
    private fun releaseDecoder() {
        runCatching { codec?.stop() }; runCatching { codec?.release() }
        codec = null; sps = null; input.clear(); packets.clear()
    }
    fun awaitClosed() { thread.join() }
    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        handler.post { releaseDecoder(); textures.stopListening(); surface.release(); textures.dispose(); thread.quitSafely() }
    }
}
