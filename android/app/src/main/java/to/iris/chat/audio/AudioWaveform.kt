package to.iris.chat.audio

import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import java.io.File
import java.nio.ByteOrder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlin.math.sqrt

internal object AudioWaveform {
    const val BAR_COUNT = 47

    // Decode locally without an audio output. Only 47 RMS values are retained.
    suspend fun decode(file: File): List<Float> = withContext(Dispatchers.Default) {
        val extractor = MediaExtractor()
        var decoder: MediaCodec? = null
        try {
            extractor.setDataSource(file.path)
            val track = (0 until extractor.trackCount).firstOrNull {
                extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: return@withContext emptyList()
            extractor.selectTrack(track)
            val format = extractor.getTrackFormat(track)
            val duration = format.getLong(MediaFormat.KEY_DURATION)
            if (duration <= 0) return@withContext emptyList()
            val codec = MediaCodec.createDecoderByType(checkNotNull(format.getString(MediaFormat.KEY_MIME)))
            decoder = codec
            codec.configure(format, null, null, 0)
            codec.start()
            var rate = format.getInteger(MediaFormat.KEY_SAMPLE_RATE)
            var channels = format.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
            var encoding = AudioFormat.ENCODING_PCM_16BIT
            val sums = DoubleArray(BAR_COUNT)
            val counts = IntArray(BAR_COUNT)
            val info = MediaCodec.BufferInfo()
            var inputEnded = false
            val deadline = System.nanoTime() + 3_000_000_000L
            while (System.nanoTime() < deadline) {
                ensureActive()
                if (!inputEnded) {
                    val input = codec.dequeueInputBuffer(10_000)
                    if (input >= 0) {
                        val size = extractor.readSampleData(checkNotNull(codec.getInputBuffer(input)), 0)
                        if (size < 0) {
                            codec.queueInputBuffer(input, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                            inputEnded = true
                        } else {
                            codec.queueInputBuffer(input, 0, size, extractor.sampleTime, 0)
                            extractor.advance()
                        }
                    }
                }
                val output = codec.dequeueOutputBuffer(info, 10_000)
                if (output == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    val decoded = codec.outputFormat
                    rate = decoded.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                    channels = decoded.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                    if (decoded.containsKey(MediaFormat.KEY_PCM_ENCODING)) encoding = decoded.getInteger(MediaFormat.KEY_PCM_ENCODING)
                    if (encoding != AudioFormat.ENCODING_PCM_16BIT && encoding != AudioFormat.ENCODING_PCM_FLOAT) return@withContext emptyList()
                } else if (output >= 0) {
                    val buffer = checkNotNull(codec.getOutputBuffer(output)).order(ByteOrder.LITTLE_ENDIAN)
                    buffer.position(info.offset)
                    buffer.limit(info.offset + info.size)
                    val bytes = if (encoding == AudioFormat.ENCODING_PCM_FLOAT) 4 else 2
                    var sample = 0
                    while (buffer.remaining() >= bytes) {
                        val value = if (bytes == 4) buffer.float.toDouble() else buffer.short / 32768.0
                        val time = info.presentationTimeUs + (sample / channels) * 1_000_000L / rate
                        val bin = (time * BAR_COUNT / duration).toInt().coerceIn(0, BAR_COUNT - 1)
                        if (value.isFinite()) { sums[bin] += value * value; counts[bin]++ }
                        sample++
                    }
                    codec.releaseOutputBuffer(output, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                        val rms = sums.mapIndexed { i, sum -> sqrt(sum / counts[i].coerceAtLeast(1)) }
                        val maximum = rms.maxOrNull()?.coerceAtLeast(0.0001) ?: 1.0
                        return@withContext rms.map { (it / maximum).toFloat() }
                    }
                }
            }
            emptyList()
        } catch (error: kotlinx.coroutines.CancellationException) {
            throw error
        } catch (_: Exception) {
            // Analysis is optional; the regular player may support more files.
            emptyList()
        } finally {
            decoder?.let { runCatching { it.stop() }; it.release() }
            extractor.release()
        }
    }
}
