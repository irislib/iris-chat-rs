package to.iris.chat.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.net.Uri
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.longClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.ByteArrayOutputStream
import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class ImageViewerClipboardTest {
    @get:Rule val compose = createComposeRule()

    @Test fun longPressCopiesImageContentsAndCanStageThemAgain() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val bitmap = Bitmap.createBitmap(2, 1, Bitmap.Config.ARGB_8888).apply {
            setPixel(0, 0, android.graphics.Color.RED); setPixel(1, 0, android.graphics.Color.GREEN)
        }
        val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        bitmap.recycle()
        val attachment = MessageAttachmentSnapshot("fixture", "colors.png", "colors.png", "htree://fixture/colors.png", true, false, false)
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        compose.setContent { IrisChatTheme {
            ImageViewerDialog(ImageViewerItem(listOf(attachment), 0, bytes, "Alex", 0), { bytes }, {}, {})
        } }
        var previous: ClipData? = null
        compose.runOnIdle { previous = clipboard.primaryClip; clipboard.clearPrimaryClip() }
        try {
            compose.waitUntil(5_000) { compose.onAllNodes(androidx.compose.ui.test.hasContentDescription("colors.png")).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithContentDescription("colors.png").performTouchInput { longClick() }
            compose.onNodeWithText("Copy image").assertExists()
            compose.waitForIdle()
            android.os.SystemClock.sleep(250)
            val screenshots = checkNotNull(context.getExternalFilesDir("screenshots")).apply { mkdirs() }
            InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let { image ->
                File(screenshots, "image-copy-menu.png").outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
                image.recycle()
            }
            compose.onNodeWithText("Copy image").performClick()
            var copiedUri: Uri? = null
            compose.waitUntil(5_000) {
                compose.runOnIdle { copiedUri = clipboard.primaryClip?.takeIf { it.description.label == "Image" }?.getItemAt(0)?.uri }
                copiedUri != null
            }
            assertEquals("content", copiedUri?.scheme)
            assertArrayEquals(bytes, context.contentResolver.openInputStream(checkNotNull(copiedUri))!!.use { it.readBytes() })
            val clip = ClipData.newUri(context.contentResolver, "Image", copiedUri)
            clip.addItem(ClipData.Item("This plain text is not a file"))
            assertEquals(listOf(copiedUri), droppedAttachmentUris(clip))
            val staged = checkNotNull(copySharedAttachmentToCache(context, checkNotNull(copiedUri)))
            assertArrayEquals(bytes, File(staged.path).readBytes())
            File(staged.path).delete()
        } finally {
            compose.runOnIdle { previous?.let(clipboard::setPrimaryClip) ?: clipboard.clearPrimaryClip() }
        }
    }
    @androidx.test.filters.SdkSuppress(minSdkVersion = 29)
    @Test fun animatedImageLongPressCopiesOriginalGif() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val bytes = android.util.Base64.decode("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7", android.util.Base64.DEFAULT)
        val attachment = MessageAttachmentSnapshot("gif-fixture", "animation.gif", "animation.gif", "htree://fixture/animation.gif", true, false, false)
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        compose.setContent { IrisChatTheme {
            ImageViewerDialog(ImageViewerItem(listOf(attachment), 0, bytes, "Alex", 0), { bytes }, {}, {})
        } }
        var previous: ClipData? = null
        compose.runOnIdle { previous = clipboard.primaryClip; clipboard.clearPrimaryClip() }
        try {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            fun webViewIn(view: android.view.View): android.webkit.WebView? {
                if (view is android.webkit.WebView) return view
                if (view is android.view.ViewGroup) for (index in 0 until view.childCount) {
                    webViewIn(view.getChildAt(index))?.let { return it }
                }
                return null
            }
            var centerX = 0f
            var centerY = 0f
            compose.waitUntil(5_000) {
                var ready = false
                instrumentation.runOnMainSync {
                    val webView = android.view.inspector.WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull(::webViewIn)
                    if (webView != null && webView.progress == 100 && webView.width > 0) {
                        val position = IntArray(2)
                        webView.getLocationOnScreen(position)
                        centerX = position[0] + webView.width / 2f
                        centerY = position[1] + webView.height / 2f
                        ready = true
                    }
                }
                ready
            }
            val down = android.os.SystemClock.uptimeMillis()
            fun touch(action: Int) {
                val event = android.view.MotionEvent.obtain(down, android.os.SystemClock.uptimeMillis(), action, centerX, centerY, 0)
                event.source = android.view.InputDevice.SOURCE_TOUCHSCREEN
                instrumentation.uiAutomation.injectInputEvent(event, true)
                event.recycle()
            }
            touch(android.view.MotionEvent.ACTION_DOWN)
            android.os.SystemClock.sleep(android.view.ViewConfiguration.getLongPressTimeout().toLong() + 200)
            touch(android.view.MotionEvent.ACTION_UP)
            compose.onNodeWithText("Copy image").performClick()
            var copied: Uri? = null
            compose.waitUntil(5_000) {
                compose.runOnIdle { copied = clipboard.primaryClip?.getItemAt(0)?.uri }
                copied != null
            }
            assertArrayEquals(bytes, context.contentResolver.openInputStream(checkNotNull(copied))!!.use { it.readBytes() })
        } finally {
            compose.runOnIdle { previous?.let(clipboard::setPrimaryClip) ?: clipboard.clearPrimaryClip() }
        }
    }

}
