package to.iris.chat.calls

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build

/** H.264 access units use Annex B throughout the FIPS call stream. */
internal object CallAvc {
    const val MAX_FRAME = 262_144
    const val MIME = "video/avc"
    private val start = byteArrayOf(0, 0, 0, 1)

    fun nals(data: ByteArray): List<ByteArray> {
        require(data.size in 1..MAX_FRAME)
        val starts = mutableListOf<Pair<Int, Int>>()
        var offset = 0
        while (offset + 3 <= data.size) {
            val size = if (data[offset] == 0.toByte() && data[offset + 1] == 0.toByte()) {
                if (data[offset + 2] == 1.toByte()) 3
                else if (offset + 4 <= data.size && data[offset + 2] == 0.toByte() && data[offset + 3] == 1.toByte()) 4 else 0
            } else 0
            if (size > 0) { starts += offset to size; offset += size } else offset++
        }
        require(starts.isNotEmpty() && starts.first().first == 0 && starts.size <= 256)
        return starts.mapIndexed { index, (at, prefix) ->
            data.copyOfRange(at + prefix, starts.getOrNull(index + 1)?.first ?: data.size).also { require(it.isNotEmpty()) }
        }
    }

    fun annexB(nals: List<ByteArray>): ByteArray = nals.fold(byteArrayOf()) { result, nal -> result + start + nal }

    fun dimensions(sps: ByteArray): Pair<Int, Int> {
        require(sps.size in 5..1024 && (sps[0].toInt() and 31) == 7)
        val bytes = ArrayList<Byte>()
        var zeroes = 0
        for (value in sps.drop(1)) {
            if (zeroes >= 2 && value == 3.toByte()) { zeroes = 0; continue }
            bytes += value
            zeroes = if (value == 0.toByte()) zeroes + 1 else 0
        }
        val bits = Bits(bytes.toByteArray())
        require(bits.read(8) == 66) // Baseline, including constrained baseline.
        bits.read(16); bits.ue(); bits.ue()
        when (bits.ue()) {
            0 -> bits.ue()
            1 -> { bits.read(1); bits.ue(); bits.ue(); repeat(bits.ue().also { require(it <= 256) }) { bits.ue() } }
            2 -> Unit
            else -> error("Invalid picture order")
        }
        bits.ue(); bits.read(1)
        val widthMbs = bits.ue() + 1
        val heightMap = bits.ue() + 1
        val frameOnly = bits.read(1)
        if (frameOnly == 0) bits.read(1)
        bits.read(1)
        var width = widthMbs * 16
        var height = heightMap * 16 * (2 - frameOnly)
        if (bits.read(1) == 1) {
            width -= (bits.ue() + bits.ue()) * 2
            height -= (bits.ue() + bits.ue()) * 2 * (2 - frameOnly)
        }
        require(width in 16..1920 && height in 16..1920 && width * height <= 1920 * 1080)
        return width to height
    }

    fun encoderFormats(width: Int, height: Int, fps: Int, bitrate: Int): List<Pair<String, MediaFormat>> =
        MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { it.isEncoder && MIME in it.supportedTypes }
            .sortedBy { if (Build.VERSION.SDK_INT >= 29) !it.isHardwareAccelerated else it.name.startsWith("OMX.google.") }
            .mapNotNull { info -> runCatching {
                val capabilities = info.getCapabilitiesForType(MIME)
                require(checkNotNull(capabilities.videoCapabilities).areSizeAndRateSupported(width, height, fps.toDouble()))
                require(MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface in capabilities.colorFormats)
                info.name to MediaFormat.createVideoFormat(MIME, width, height).apply {
                    setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                    setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                    setInteger(MediaFormat.KEY_FRAME_RATE, fps)
                    setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
                    setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
                    setInteger(MediaFormat.KEY_LEVEL, if (width * height > 1280 * 720) MediaCodecInfo.CodecProfileLevel.AVCLevel4 else MediaCodecInfo.CodecProfileLevel.AVCLevel31)
                    setInteger(MediaFormat.KEY_PRIORITY, 0)
                    if (Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                    if (checkNotNull(capabilities.encoderCapabilities).isBitrateModeSupported(MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR))
                        setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                }
            }.getOrNull() }

    private class Bits(private val bytes: ByteArray) {
        private var position = 0
        fun read(count: Int): Int {
            require(position + count <= bytes.size * 8)
            var result = 0
            repeat(count) { result = (result shl 1) or ((bytes[position / 8].toInt() shr (7 - position++ % 8)) and 1) }
            return result
        }
        fun ue(): Int {
            var zeroes = 0
            while (read(1) == 0) { zeroes++; require(zeroes < 16) }
            return (1 shl zeroes) - 1 + read(zeroes)
        }
    }
}
