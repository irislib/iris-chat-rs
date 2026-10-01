package to.iris.chat.ui.screens

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.net.Uri
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.State
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextRange
import androidx.core.content.FileProvider
import androidx.core.view.inputmethod.InputConnectionCompat
import androidx.core.view.inputmethod.InputContentInfoCompat
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import org.junit.After
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.rust.AppAction
import to.iris.chat.ui.theme.IrisChatTheme

/** Exercises Android's actual clipboard and IME input connection, not a synthetic callback. */
class ChatAttachmentPasteTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val paste = ChatAttachmentPaste(context, scope)
    private val draft = TextFieldState("Caption")
    private val attachments = mutableStateOf<List<PickedAttachment>>(emptyList())
    private val sendDirectly = mutableStateOf(false)
    private val sent = mutableListOf<AppAction>()
    private val draftUpdates = mutableListOf<String>()
    private val sourceFiles = mutableListOf<File>()
    private var previousClip: ClipData? = null
    private var clipboardChanged = false

    @After fun cleanup() {
        compose.runOnUiThread {
            paste.close()
            if (clipboardChanged) {
                val clipboard = context.getSystemService(ClipboardManager::class.java)
                previousClip?.let(clipboard::setPrimaryClip) ?: clipboard.clearPrimaryClip()
            }
        }
        runBlocking { scope.coroutineContext[Job]!!.children.toList().joinAll() }
        scope.cancel()
        attachments.value.forEach { File(it.path).delete() }
        sourceFiles.forEach(File::delete)
    }

    @Test fun clipboardStagesMultipleFilesAndLeavesOrdinaryTextPasteAtTheCaret() {
        showComposer()
        val image = sharedFile(".png", pngBytes())
        val document = sharedFile(".pdf", "%PDF-1.4\nclipboard fixture".toByteArray())
        val clip = ClipData.newUri(context.contentResolver, "Files", uri(image)).apply {
            addItem(ClipData.Item(uri(document)))
        }
        pasteClipboard(clip)
        waitForAttachments(2)
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertEquals(0, sent.size)
        assertArrayEquals(image.readBytes(), File(attachments.value[0].path).readBytes())
        assertArrayEquals(document.readBytes(), File(attachments.value[1].path).readBytes())
        compose.onNodeWithTag("chatSelectedAttachments").assertIsDisplayed()
        screenshot("android-clipboard-attachments.png")

        compose.onNodeWithTag("chatMessageInput").performTextInputSelection(TextRange(3))
        pasteClipboard(ClipData.newPlainText("Text", " pasted "))
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Cap pasted tion")
        assertEquals(2, attachments.value.size)
        assertEquals(0, sent.size)
        compose.onNodeWithTag("chatSendButton").performClick()
        val action = sent.single() as AppAction.SendAttachments
        assertEquals(2, action.attachments.size)
        assertEquals("Cap pasted tion", action.caption)
    }

    @Test fun keyboardImageStagesInDirectModeAndRemovalDeletesItsCopy() {
        sendDirectly.value = true
        showComposer()
        val image = sharedFile(".png", pngBytes())
        compose.onNodeWithTag("chatMessageInput").performClick()
        commitFromKeyboard(uri(image), "image/png")
        waitForAttachments(1)
        compose.onNodeWithTag("chatDirectFileMode").assertIsDisplayed()
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertEquals(0, sent.size)
        val staged = File(attachments.value.single().path)
        screenshot("android-ime-direct-attachments.png")
        compose.onNodeWithContentDescription("Remove attachment").performClick()
        assertFalse(staged.exists())
        commitFromKeyboard(uri(image), "image/png")
        waitForAttachments(1)
        compose.onNodeWithTag("chatSendButton").performClick()
        val action = sent.single() as AppAction.SendDirectFiles
        assertEquals(1, action.attachments.size)
        assertEquals("Caption", action.caption)
        assertTrue(File(action.attachments.single().filePath).exists())
    }

    @Test fun sendingOrLeavingDuringImportDiscardsLateCopies() {
        showComposer()
        compose.onNodeWithTag("chatMessageInput").performClick()
        val image = sharedFile(".png", pngBytes())
        val output = File(context.cacheDir, "attachments/outgoing")
        val before = output.listFiles().orEmpty().map { it.name }.toSet()
        // Both operations run on the UI thread before the asynchronous copy can
        // deliver. The first models Send; the second models leaving this chat.
        for (leave in listOf(false, true)) {
            compose.runOnIdle {
                commitFromKeyboardOnMain(uri(image), "image/png")
                if (leave) paste.close() else paste.sent(emptyList())
            }
            runBlocking { scope.coroutineContext[Job]!!.children.toList().joinAll() }
            assertTrue(attachments.value.isEmpty())
            assertEquals(before, output.listFiles().orEmpty().map { it.name }.toSet())
        }
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertTrue(sent.isEmpty())
    }

    @Test fun fileClipboardSuppressesAccompanyingTextAndRejectedContentDoesNotChangeTheDraft() {
        showComposer()
        val image = sharedFile(".png", pngBytes())
        pasteClipboard(ClipData.newUri(context.contentResolver, "Image and text", uri(image)).apply {
            addItem(ClipData.Item(" and text"))
        })
        waitForAttachments(1)
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        // Android forbids putting file:// URIs on the system clipboard before
        // our receiver runs. Use another unsupported scheme to reach the input.
        pasteClipboard(ClipData.newRawUri("Unsupported", Uri.parse("android.resource://${context.packageName}/raw/unsupported")))
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertEquals(1, attachments.value.size)
        assertTrue(sent.isEmpty())
    }

    @Test fun unreadableFileRejectsTheWholeBatchAndCleansSuccessfulCopies() {
        showComposer()
        val readable = sharedFile(".png", pngBytes())
        val unreadable = sharedFile(".pdf", byteArrayOf())
        val unreadableUri = uri(unreadable)
        unreadable.delete()
        val output = File(context.cacheDir, "attachments/outgoing")
        val before = output.listFiles().orEmpty().map { it.name }.toSet()
        pasteClipboard(ClipData.newUri(context.contentResolver, "Files", uri(readable)).apply {
            addItem(ClipData.Item(unreadableUri))
            addItem(ClipData.Item("Accompanying file-manager text"))
        })
        runBlocking { withTimeout(5_000) { scope.coroutineContext[Job]!!.children.toList().joinAll() } }
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertTrue(attachments.value.isEmpty())
        assertEquals(before, output.listFiles().orEmpty().map { it.name }.toSet())
        assertTrue(sent.isEmpty())
    }

    @Test fun unsupportedUriRejectsTheWholeFileBatchBeforeMakingCopies() {
        showComposer()
        val readable = sharedFile(".png", pngBytes())
        val output = File(context.cacheDir, "attachments/outgoing")
        val before = output.listFiles().orEmpty().map { it.name }.toSet()
        pasteClipboard(ClipData.newUri(context.contentResolver, "Files", uri(readable)).apply {
            addItem(ClipData.Item(Uri.parse("android.resource://${context.packageName}/raw/unsupported")))
            addItem(ClipData.Item("File labels"))
        })
        compose.onNodeWithTag("chatMessageInput").assertTextEquals("Caption")
        assertTrue(attachments.value.isEmpty())
        assertFalse(scope.coroutineContext[Job]!!.children.any())
        assertEquals(before, output.listFiles().orEmpty().map { it.name }.toSet())
        assertTrue(sent.isEmpty())
    }

    @Test fun largeTextPasteIsOneDraftEditAndDoesNoAttachmentWork() {
        showComposer()
        val text = "A paragraph pasted from another app.\n".repeat(2_800)
        val expected = "Caption$text"
        val output = File(context.cacheDir, "attachments/outgoing")
        val before = output.listFiles().orEmpty().map { it.name }.toSet()
        val started = SystemClock.uptimeMillis()
        pasteClipboard(ClipData.newPlainText("Text", text))
        compose.waitUntil(5_000) { draftUpdates.lastOrNull() == expected }
        val elapsedMs = SystemClock.uptimeMillis() - started
        File(checkNotNull(context.getExternalFilesDir("screenshots")), "android-clipboard-large-text.json").apply {
            parentFile?.mkdirs()
            writeText("{\"characters\":${text.length},\"elapsed_ms\":$elapsedMs,\"draft_updates\":${draftUpdates.size}}")
        }
        assertTrue("A 100 KB text paste should finish promptly", elapsedMs < 5_000)
        assertEquals(listOf(expected), draftUpdates)
        assertEquals(expected, draft.text.toString())
        assertTrue(attachments.value.isEmpty())
        assertFalse(scope.coroutineContext[Job]!!.children.any())
        assertEquals(before, output.listFiles().orEmpty().map { it.name }.toSet())
        assertTrue(sent.isEmpty())
    }

    @Test fun removingOnlyTheComposerCancelsImportAndReopeningKeepsTheDraftUsable() {
        val copied = CountDownLatch(1)
        val resume = CountDownLatch(1)
        val receiver = ChatAttachmentPaste(context, scope) { ctx, uri ->
            val attachment = copySharedAttachmentToCache(ctx, uri)
            copied.countDown()
            check(resume.await(5, TimeUnit.SECONDS)) { "Receiver-disposal test did not release copy" }
            attachment
        }
        val visible = mutableStateOf(true)
        val output = File(context.cacheDir, "attachments/outgoing")
        val before = output.listFiles().orEmpty().map { it.name }.toSet()
        try {
            showComposer(receiver, visible)
            val image = sharedFile(".png", pngBytes())
            commitFromKeyboard(uri(image), "image/png")
            assertTrue("Clipboard copy started", copied.await(5, TimeUnit.SECONDS))
            compose.runOnIdle { visible.value = false }
            compose.waitForIdle()
            compose.onNodeWithTag("chatMessageInput").assertDoesNotExist()
            // Keep the screen's receiver owner alive, as blocking a chat does.
            // Only the composer's DisposableEffect may invalidate this import.
            resume.countDown()
            runBlocking { withTimeout(5_000) { scope.coroutineContext[Job]!!.children.toList().joinAll() } }
            assertTrue(attachments.value.isEmpty())
            assertEquals(before, output.listFiles().orEmpty().map { it.name }.toSet())
            assertEquals("Caption", draft.text.toString())
            assertTrue(sent.isEmpty())
            compose.runOnIdle { visible.value = true }
            compose.onNodeWithTag("chatMessageInput").performClick().assertTextEquals("Caption")
            commitFromKeyboard(uri(image), "image/png")
            waitForAttachments(1)
            assertTrue(sent.isEmpty())
        } finally {
            resume.countDown()
            compose.runOnUiThread { receiver.close() }
        }
    }

    private fun showComposer(receiver: ChatAttachmentPaste = paste, visible: State<Boolean> = mutableStateOf(true)) {
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                Column(Modifier.fillMaxSize()) {
                    Spacer(Modifier.weight(1f))
                    if (visible.value) ComposerBar(draft, attachments.value, false, false, null,
                        inputContentModifier = receiver.receiverModifier(true, sendDirectly.value) {
                            attachments.value += it
                        },
                        onDraftChange = { draftUpdates.add(it) }, onAttach = {},
                        onRemoveAttachment = { receiver.remove(it); attachments.value -= it },
                        onSend = {
                            sent += attachmentSendAction("paste-chat", attachments.value, draft.text.toString(), sendDirectly.value)
                            receiver.sent(attachments.value)
                        }, sendFilesDirectly = sendDirectly.value)
                }
            }
        }
        compose.onNodeWithTag("chatMessageInput").performClick().performTextInputSelection(TextRange(draft.text.length))
    }

    private fun pasteClipboard(clip: ClipData) {
        compose.runOnIdle {
            val clipboard = context.getSystemService(ClipboardManager::class.java)
            if (!clipboardChanged) { previousClip = clipboard.primaryClip; clipboardChanged = true }
            clipboard.setPrimaryClip(clip)
        }
        compose.onNodeWithTag("chatMessageInput").performKeyInput {
            keyDown(Key.CtrlLeft); pressKey(Key.V); keyUp(Key.CtrlLeft)
        }
        compose.waitForIdle()
    }

    private fun commitFromKeyboard(uri: Uri, mime: String) {
        compose.runOnIdle { commitFromKeyboardOnMain(uri, mime) }
    }

    private fun commitFromKeyboardOnMain(uri: Uri, mime: String) {
        val view = checkNotNull(findTextEditor(compose.activity.window.decorView))
        val info = EditorInfo()
        val connection = checkNotNull(view.onCreateInputConnection(info))
        assertTrue("The real input connection must advertise rich content", info.contentMimeTypes?.isNotEmpty() == true)
        assertTrue(InputConnectionCompat.commitContent(connection, info,
            InputContentInfoCompat(uri, ClipDescription("Keyboard attachment", arrayOf(mime)), null),
            InputConnectionCompat.INPUT_CONTENT_GRANT_READ_URI_PERMISSION, null))
    }

    private fun findTextEditor(view: View): View? {
        if (view.onCheckIsTextEditor()) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) findTextEditor(view.getChildAt(i))?.let { return it }
        return null
    }

    private fun waitForAttachments(count: Int) {
        compose.waitUntil(5_000) { attachments.value.size == count }
    }

    private fun sharedFile(suffix: String, bytes: ByteArray): File =
        File.createTempFile("paste-", suffix, File(context.cacheDir, "attachments/share").apply { mkdirs() })
            .also { it.writeBytes(bytes); sourceFiles.add(it) }

    private fun uri(file: File): Uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)

    private fun pngBytes(): ByteArray {
        val image = Bitmap.createBitmap(24, 24, Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.CYAN) }
        return java.io.ByteArrayOutputStream().use { output ->
            image.compress(Bitmap.CompressFormat.PNG, 100, output)
            image.recycle()
            output.toByteArray()
        }
    }

    private fun screenshot(name: String) {
        val dir = checkNotNull(context.getExternalFilesDir("screenshots")).apply { mkdirs() }
        InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let { bitmap ->
            File(dir, name).outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
    }
}
