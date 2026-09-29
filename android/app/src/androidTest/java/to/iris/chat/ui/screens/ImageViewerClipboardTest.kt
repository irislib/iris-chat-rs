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
        compose.waitUntil(5_000) { compose.onAllNodes(androidx.compose.ui.test.hasContentDescription("colors.png")).fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithContentDescription("colors.png").performTouchInput { longClick() }
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
    }
}
