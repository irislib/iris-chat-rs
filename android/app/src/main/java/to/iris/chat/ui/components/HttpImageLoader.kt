package to.iris.chat.ui.components

import android.graphics.Bitmap
import androidx.compose.runtime.compositionLocalOf
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.imageLoadUrls

data class ImageLoadRequest(val originalUrl: String, val urls: List<String>, val allowOriginalRedirects: Boolean)

val LocalImagePreferences = compositionLocalOf<PreferencesSnapshot?> { null }

fun imageLoadRequest(
    originalSrc: String,
    preferences: PreferencesSnapshot,
    width: UInt,
    height: UInt,
    square: Boolean,
): ImageLoadRequest = ImageLoadRequest(
    originalSrc.trim(),
    imageLoadUrls(originalSrc, preferences, width, height, square),
    !preferences.imageProxyEnabled || preferences.imageProxyFallbackEnabled,
)

internal suspend fun loadHttpImage(
    request: ImageLoadRequest,
    cached: (String) -> Bitmap? = { null },
    store: (String, Bitmap) -> Unit = { _, _ -> },
    decode: (ByteArray) -> Bitmap?,
): Bitmap? {
    for (url in request.urls) {
        currentCoroutineContext().ensureActive()
        cached(url)?.let { return it }
        val data = try {
            readImage(url, allowCrossOriginRedirects = url == request.originalUrl && request.allowOriginalRedirects)
        } catch (_: IOException) {
            null
        }
        currentCoroutineContext().ensureActive()
        val bitmap = data?.let(decode) ?: continue
        store(url, bitmap)
        return bitmap
    }
    return null
}

private fun readImage(source: String, allowCrossOriginRedirects: Boolean): ByteArray? {
    var url = URL(source)
    val initialUrl = url
    repeat(6) {
        if (url.protocol != "http" && url.protocol != "https") return null
        val connection = url.openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 15_000
            connection.readTimeout = 15_000
            connection.instanceFollowRedirects = false
            val status = connection.responseCode
            if (status in 200..299) return connection.inputStream.use { it.readBytes() }
            if (status !in listOf(301, 302, 303, 307, 308)) return null
            val location = connection.getHeaderField("Location") ?: return null
            val redirected = URL(url, location)
            if (!allowCrossOriginRedirects && !sameOrigin(initialUrl, redirected)) return null
            url = redirected
        } finally {
            connection.disconnect()
        }
    }
    return null
}

private fun sameOrigin(first: URL, second: URL): Boolean =
    first.protocol.equals(second.protocol, ignoreCase = true) &&
        first.host.equals(second.host, ignoreCase = true) &&
        (if (first.port == -1) first.defaultPort else first.port) ==
        (if (second.port == -1) second.defaultPort else second.port)
