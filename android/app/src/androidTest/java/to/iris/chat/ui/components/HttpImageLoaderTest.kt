package to.iris.chat.ui.components

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.ByteArrayOutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.SocketException
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.Rule
import org.junit.runner.RunWith
import to.iris.chat.rust.buildLargeTestAppState
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class HttpImageLoaderTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun raw_avatar_uses_current_proxy_preferences_and_fails_closed_before_preferences_load() {
        ImageServer().use { original ->
            ImageServer("status").use { proxy ->
                val preferences = mutableStateOf<PreferencesSnapshot?>(null)
                composeRule.setContent {
                    CompositionLocalProvider(LocalImagePreferences provides preferences.value) {
                        IrisChatTheme(darkTheme = false) { IrisAvatar(label = "Test", imageUrl = original.url) }
                    }
                }
                composeRule.waitForIdle()
                assertEquals(0, original.requests.get())
                composeRule.runOnIdle {
                    preferences.value = buildLargeTestAppState(0u, 0u, 0u).preferences.copy(imageProxyUrl = proxy.url)
                }
                composeRule.waitUntil(5_000) { proxy.requests.get() > 0 }
                assertEquals(0, original.requests.get())
                composeRule.runOnIdle {
                    preferences.value = preferences.value!!.copy(imageProxyFallbackEnabled = true)
                }
                composeRule.waitUntil(5_000) { original.requests.get() == 1 }
                composeRule.runOnIdle {
                    preferences.value = preferences.value!!.copy(imageProxyFallbackEnabled = false)
                }
                composeRule.waitUntil(5_000) { proxy.requests.get() >= 3 }
                assertEquals(1, original.requests.get())
            }
        }
    }

    @Test
    fun successful_proxy_never_contacts_original_even_when_fallback_enabled() = runBlocking {
        ImageServer().use { original ->
            ImageServer().use { proxy ->
                assertNotNull(loadHttpImage(request(proxy, original, fallback = true), decode = ::decode))
                assertEquals(1, proxy.requests.get())
                assertEquals(0, original.requests.get())
            }
        }
    }

    @Test
    fun failed_proxy_contacts_original_only_after_opt_in() = runBlocking {
        for (failure in listOf("status", "decode", "disconnect")) {
            ImageServer().use { original ->
                ImageServer(failure).use { proxy ->
                    assertNull(loadHttpImage(request(proxy, original, fallback = false), decode = ::decode))
                    assertTrue(proxy.requests.get() > 0)
                    assertEquals(0, original.requests.get())
                    assertNotNull(loadHttpImage(request(proxy, original, fallback = true), decode = ::decode))
                    assertEquals(1, original.requests.get())
                }
            }
        }
    }

    @Test
    fun proxy_redirect_cannot_contact_original_without_opt_in() = runBlocking {
        ImageServer().use { original ->
            ImageServer("redirect", original.url).use { proxy ->
                assertNull(loadHttpImage(request(proxy, original, fallback = false), decode = ::decode))
                assertEquals(0, original.requests.get())
                val preferences = buildLargeTestAppState(0u, 0u, 0u).preferences.copy(imageProxyUrl = proxy.url)
                val alreadyProxied = imageLoadRequest(proxy.url, preferences, 80u, 80u, true)
                assertEquals(listOf(proxy.url), alreadyProxied.urls)
                assertNull(loadHttpImage(alreadyProxied, decode = ::decode))
                assertEquals(0, original.requests.get())
                assertNotNull(loadHttpImage(request(proxy, original, fallback = true), decode = ::decode))
                assertEquals(1, original.requests.get())
            }
        }
    }

    @Test
    fun original_redirects_still_work_when_proxy_is_disabled() = runBlocking {
        ImageServer().use { destination ->
            ImageServer("redirect", destination.url).use { original ->
                val preferences = buildLargeTestAppState(0u, 0u, 0u).preferences.copy(imageProxyEnabled = false)
                val request = imageLoadRequest(original.url, preferences, 80u, 80u, true)
                assertNotNull(loadHttpImage(request, decode = ::decode))
                assertEquals(1, destination.requests.get())
            }
        }
    }

    @Test
    fun invalid_proxy_configuration_cannot_contact_original_without_opt_in() = runBlocking {
        ImageServer().use { original ->
            val preferences = buildLargeTestAppState(0u, 0u, 0u).preferences.copy(imageProxyUrl = "invalid proxy URL")
            assertNull(loadHttpImage(imageLoadRequest(original.url, preferences, 80u, 80u, true), decode = ::decode))
            assertEquals(0, original.requests.get())
            preferences.imageProxyFallbackEnabled = true
            assertNotNull(loadHttpImage(imageLoadRequest(original.url, preferences, 80u, 80u, true), decode = ::decode))
            assertEquals(1, original.requests.get())
        }
    }

    private fun request(proxy: ImageServer, original: ImageServer, fallback: Boolean): ImageLoadRequest {
        val preferences = buildLargeTestAppState(0u, 0u, 0u).preferences.copy(
            imageProxyEnabled = true,
            imageProxyUrl = proxy.url,
            imageProxyFallbackEnabled = fallback,
        )
        return imageLoadRequest(original.url, preferences, 80u, 80u, true)
    }

    private fun decode(bytes: ByteArray): Bitmap? = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)

    private class ImageServer(private val mode: String = "image", private val redirect: String = "") : AutoCloseable {
        private val server = ServerSocket(0, 10, InetAddress.getByName("127.0.0.1"))
        val url = "http://127.0.0.1:${server.localPort}"
        val requests = AtomicInteger()
        private val png = ByteArrayOutputStream().use { stream ->
            Bitmap.createBitmap(2, 2, Bitmap.Config.ARGB_8888).apply {
                eraseColor(android.graphics.Color.RED)
                compress(Bitmap.CompressFormat.PNG, 100, stream)
                recycle()
            }
            stream.toByteArray()
        }
        private val worker = thread(isDaemon = true) {
            while (!server.isClosed) {
                try {
                    server.accept().use { socket ->
                        socket.soTimeout = 5_000
                        val reader = socket.getInputStream().bufferedReader()
                        while (!reader.readLine().isNullOrEmpty()) { }
                        requests.incrementAndGet()
                        if (mode != "disconnect") {
                            val body = if (mode == "decode") "invalid image".toByteArray() else png
                            val status = when (mode) { "status" -> 503; "redirect" -> 302; else -> 200 }
                            val location = if (mode == "redirect") "Location: $redirect\r\n" else ""
                            socket.getOutputStream().apply {
                                write("HTTP/1.1 $status Test\r\n${location}Content-Length: ${body.size}\r\nConnection: close\r\n\r\n".toByteArray())
                                write(body)
                                flush()
                            }
                        }
                    }
                } catch (_: SocketException) {
                    if (!server.isClosed) throw AssertionError("Image server failed")
                }
            }
        }

        override fun close() {
            server.close()
            worker.join(5_000)
        }
    }
}
