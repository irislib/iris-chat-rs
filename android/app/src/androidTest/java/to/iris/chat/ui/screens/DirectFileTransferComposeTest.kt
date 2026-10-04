package to.iris.chat.ui.screens

import android.graphics.Bitmap
import android.app.Activity
import android.content.Intent
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.compose.runtime.CompositionLocalProvider
import androidx.core.app.ActivityOptionsCompat
import android.net.Uri
import androidx.compose.material3.Surface
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DirectFileSnapshot
import to.iris.chat.rust.DirectFileTransferSnapshot
import to.iris.chat.rust.DirectFileTransferStatus
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class DirectFileTransferComposeTest {
    @get:Rule val composeRule = createComposeRule()

    @Test
    fun cancellingDestinationPickerLeavesOfferUnacceptedAndAllowsDecline() {
        val actions = mutableListOf<AppAction>()
        val acceptedIds = mutableListOf<String>()
        var pickers = 0
        val registry = object : ActivityResultRegistry() {
            override fun <I, O> onLaunch(requestCode: Int, contract: ActivityResultContract<I, O>, input: I, options: ActivityOptionsCompat?) {
                pickers += 1
                dispatchResult(requestCode, Activity.RESULT_CANCELED, Intent())
            }
        }
        val owner = object : ActivityResultRegistryOwner { override val activityResultRegistry = registry }
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) {
                CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                    Surface { ChatDirectFileTransfer(fixture(isSender = false), "self-chat",
                        accept = { _, id, _ -> acceptedIds.add(id) }, dispatch = actions::add) }
                }
            }
        }
        composeRule.onNodeWithText("Weekend photos.zip").assertIsDisplayed()
        composeRule.onNodeWithText("Packing list.txt").assertIsDisplayed()
        composeRule.onNodeWithTag("chatDirectTransferCancel-test-transfer").assertDoesNotExist()
        composeRule.onNodeWithTag("chatDirectTransferAccept-test-transfer").performClick()
        composeRule.runOnIdle {
            assertEquals(1, pickers)
            assertTrue(acceptedIds.isEmpty())
            assertTrue("Cancelling the save picker must not accept or decline the offer", actions.isEmpty())
        }
        composeRule.onNodeWithTag("chatDirectTransferDecline-test-transfer").performClick()
        assertEquals(AppAction.DeclineDirectFiles("self-chat", "test-transfer"), actions.last())
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val output = File(context.getExternalFilesDir("screenshots"), "direct-file-offer.png")
        output.parentFile?.mkdirs()
        val image = composeRule.onNodeWithTag("chatDirectTransfer-test-transfer").captureToImage().asAndroidBitmap()
        output.outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    @Test
    fun senderDeviceCanCancelButCannotAccept() {
        val actions = mutableListOf<AppAction>()
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) {
                ChatDirectFileTransfer(fixture(isSender = true), "self-chat", dispatch = actions::add)
            }
        }
        composeRule.onNodeWithTag("chatDirectTransferAccept-test-transfer").assertDoesNotExist()
        composeRule.onNodeWithTag("chatDirectTransferCancel-test-transfer").performClick()
        assertEquals(AppAction.CancelDirectFiles("self-chat", "test-transfer"), actions.last())
    }

    @Test
    fun addingFilesKeepsExistingDirectFilesOffUploads() {
        assertTrue(directFileSendMode(current = true, hasFiles = true, selectedDirectly = false))
        assertTrue(directFileSendMode(current = false, hasFiles = true, selectedDirectly = true))
        assertFalse(directFileSendMode(current = true, hasFiles = false, selectedDirectly = false))
    }

    @Test
    fun sendDirectlyChoiceBuildsOneOfferForMultipleFiles() {
        var action: AppAction? = null
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val directory = File(context.cacheDir, "direct-file-test-input").apply { mkdirs() }
        val sources = listOf("one.txt", "two.pdf").map { name -> File(directory, name).apply { writeText(name) } }
        val files = sources.map { checkNotNull(copyAttachmentToCache(context, Uri.fromFile(it))) }
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) {
                ChatAttachmentSourceDialog(onDismiss = {}) { directly ->
                    action = attachmentSendAction("self-chat", files, "For my laptop", directly)
                }
            }
        }
        composeRule.onNodeWithTag("chatDirectFileButton").assertIsDisplayed()
        val screenshot = composeRule.onNode(isDialog()).captureToImage().asAndroidBitmap()
        val menuImage = File(context.getExternalFilesDir("screenshots"), "direct-file-source-menu.png")
        menuImage.parentFile?.mkdirs()
        menuImage.outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        composeRule.onNodeWithTag("chatDirectFileButton").performClick()
        val offer = action as AppAction.SendDirectFiles
        assertEquals("self-chat", offer.chatId)
        assertEquals(files.map { it.filename }, offer.attachments.map { it.filename })
        assertEquals(files.map { it.path }, offer.attachments.map { it.filePath })
        assertEquals("For my laptop", offer.caption)
        files.forEach { assertEquals(it.filename, File(it.path).readText()); File(it.path).delete() }
        sources.forEach { it.delete() }
        directory.delete()
    }

    @Test
    fun directFilesStayInTheScrollableSourceRowOnNarrowScreens() {
        var direct: Boolean? = null
        composeRule.setContent {
            IrisChatTheme(darkTheme = false) {
                Surface {
                    Box(Modifier.width(180.dp)) { ChatAttachmentSourceRow { direct = it } }
                }
            }
        }
        val files = composeRule.onNodeWithTag("chatAttachmentFilesButton")
        files.assertIsDisplayed()
        val fileTop = files.fetchSemanticsNode().boundsInRoot.top
        val send = composeRule.onNodeWithTag("chatDirectFileButton")
        send.performScrollTo().assertIsDisplayed()
        assertEquals(fileTop, send.fetchSemanticsNode().boundsInRoot.top, 1f)
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val output = File(context.getExternalFilesDir("screenshots"), "direct-file-source-row.png")
        output.parentFile?.mkdirs()
        val image = composeRule.onNodeWithTag("chatAttachmentSources").captureToImage().asAndroidBitmap()
        output.outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        send.performClick()
        assertEquals(true, direct)
    }

    private fun fixture(isSender: Boolean) = DirectFileTransferSnapshot(
        id = "test-transfer",
        files = listOf(
            DirectFileSnapshot("Weekend photos.zip", 1_024_000uL, null),
            DirectFileSnapshot("Packing list.txt", 512uL, null),
        ),
        status = DirectFileTransferStatus.OFFERED,
        isSender = isSender,
        transferredBytes = 0uL,
        totalBytes = 1_024_512uL,
        error = null,
    )
}
