package to.iris.chat.ui.screens

import android.content.Context
import android.content.ContextWrapper
import android.graphics.Bitmap
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.espresso.Espresso.closeSoftKeyboard
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.UUID
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.core.AppManager
import to.iris.chat.core.MockRustAppClient
import to.iris.chat.core.RecordingSecureSecretStore
import to.iris.chat.rust.*
import to.iris.chat.ui.theme.IrisChatTheme

class ChatKeyboardTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun keyboardLiftsLatestAndOlderMessagesWithoutLosingTheirPosition() {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(base.cacheDir, "keyboard-${UUID.randomUUID()}").apply { mkdirs() }
        val context = object : ContextWrapper(base) {
            override fun getApplicationContext(): Context = this
            override fun getFilesDir() = File(root, "files").apply { mkdirs() }
            override fun getCacheDir() = File(root, "cache").apply { mkdirs() }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val state = buildLargeTestAppState(1u, 0u, 40u)
        val chat = state.currentChat!!.copy(directChatCapability = DirectChatCapabilityState.AVAILABLE, isRequest = false,
            messages = state.currentChat!!.messages.mapIndexed { index, message -> message.copy(
                body = "Message $index: This conversation stays in view while I write a reply.", authorPictureUrl = null,
            ) })
        val rust = MockRustAppClient(state.copy(currentChat = chat, router = Router(Screen.ChatList, listOf(Screen.Chat(chat.chatId)))))
        val store = PreferenceDataStoreFactory.create(scope = scope) { File(root, "test.preferences_pb") }
        val manager = AppManager(context, scope, RecordingSecureSecretStore(), dataStore = store,
            rustFactory = { _, _ -> rust }, clearDeliveredNotifications = {})
        try {
            compose.activityRule.scenario.onActivity { activity ->
                activity.enableEdgeToEdge()
                activity.window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
            }
            compose.setContent { IrisChatTheme(darkTheme = false) { ChatScreen(manager, chat.chatId) } }
            val timeline = compose.onNodeWithTag("chatTimeline")
            val input = compose.onNodeWithTag("chatMessageInput")
            val latestText = chat.messages.last().body
            val latest = compose.onNodeWithText(latestText)
            try {
                compose.waitUntil(10_000) { compose.onAllNodesWithText(latestText).fetchSemanticsNodes().isNotEmpty() }
                latest.assertIsDisplayed()
                val latestBubble = compose.onNodeWithTag("chatMessage-${chat.messages.last().id}")
                val geometry = {
                    Triple(timeline.getUnclippedBoundsInRoot(), latestBubble.getUnclippedBoundsInRoot(), input.getUnclippedBoundsInRoot())
                }
                var previous = geometry()
                var stableAt = System.nanoTime()
                // A displayed text node can still have its bubble's timestamp
                // clipped. Let production initial scrolling settle without
                // scrolling here or opening the keyboard to repair its position.
                compose.waitUntil(5_000) {
                    val next = geometry()
                    if (next != previous) { previous = next; stableAt = System.nanoTime() }
                    val (viewport, bubble, composer) = next
                    bubble.top >= viewport.top && bubble.bottom <= viewport.bottom &&
                        viewport.bottom <= composer.top && System.nanoTime() - stableAt > 250_000_000
                }
                compose.waitForIdle()
            } catch (failure: Throwable) {
                screenshot("android-initial-failure")
                File(base.getExternalFilesDir("screenshots"), "android-initial-semantics.txt").writeText(
                    "Current chat: ${manager.currentChat.value?.chatId}; messages: ${manager.currentChat.value?.messages?.size}\n" + compose.onRoot().printToString(),
                )
                throw failure
            }
            latest.assertIsDisplayed()
            screenshot("android-latest-keyboard-hidden-before")
            input.performClick()
            waitForKeyboard(true)
            latest.assertIsDisplayed()
            assertTrue("The complete latest bubble must stay above the composer", compose.onNodeWithTag("chatMessage-${chat.messages.last().id}").getUnclippedBoundsInRoot().bottom <= timeline.getUnclippedBoundsInRoot().bottom)
            assertTrue(latest.fetchSemanticsNode().boundsInRoot.bottom <= input.fetchSemanticsNode().boundsInRoot.top)
            screenshot("android-latest-keyboard-open")
            val incoming = chat.messages.last().copy(id = "keyboard-new-message", body = "A new message arrives while the keyboard is open.", createdAtSecs = chat.messages.last().createdAtSecs + 1uL)
            val updated = rust.currentState.copy(rev = rust.currentState.rev + 1uL, currentChat = chat.copy(messages = chat.messages + incoming))
            compose.runOnIdle { rust.currentState = updated; rust.emit(AppUpdate.FullState(updated)) }
            compose.waitUntil(5_000) { compose.onAllNodesWithText(incoming.body).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText(incoming.body).assertIsDisplayed()
            assertTrue(compose.onNodeWithText(incoming.body).fetchSemanticsNode().boundsInRoot.bottom <= input.fetchSemanticsNode().boundsInRoot.top)
            assertTrue(compose.onNodeWithTag("chatMessage-${incoming.id}").getUnclippedBoundsInRoot().bottom <= timeline.getUnclippedBoundsInRoot().bottom)
            screenshot("android-new-message-keyboard-open")
            closeSoftKeyboard()
            waitForKeyboard(false)
            latest.assertIsDisplayed()
            screenshot("android-latest-keyboard-hidden")

            timeline.performScrollToIndex(18)
            val reading = compose.onNodeWithText(chat.messages[23].body)
            reading.assertIsDisplayed()
            val beforeY = reading.fetchSemanticsNode().boundsInRoot.top
            val distance = input.fetchSemanticsNode().boundsInRoot.top - beforeY
            screenshot("android-older-keyboard-hidden-before")
            input.performClick()
            waitForKeyboard(true)
            reading.assertIsDisplayed()
            assertEquals(distance, input.fetchSemanticsNode().boundsInRoot.top - reading.fetchSemanticsNode().boundsInRoot.top, 3f)
            compose.onNodeWithText(latestText).assertDoesNotExist()
            screenshot("android-older-keyboard-open")
            val later = incoming.copy(id = "keyboard-later-message", body = "Another message while reading history.", createdAtSecs = incoming.createdAtSecs + 1uL)
            val olderUpdate = updated.copy(rev = updated.rev + 1uL, currentChat = updated.currentChat!!.copy(messages = updated.currentChat!!.messages + later))
            compose.runOnIdle { rust.currentState = olderUpdate; rust.emit(AppUpdate.FullState(olderUpdate)) }
            compose.waitUntil(5_000) { manager.currentChat.value?.messages?.size == 42 }
            compose.waitForIdle()
            assertEquals(distance, input.fetchSemanticsNode().boundsInRoot.top - reading.fetchSemanticsNode().boundsInRoot.top, 3f)
            compose.onNodeWithText(later.body).assertDoesNotExist()
            screenshot("android-older-incoming-keyboard-open")
            closeSoftKeyboard()
            waitForKeyboard(false)
            assertEquals(beforeY, reading.fetchSemanticsNode().boundsInRoot.top, 3f)
            screenshot("android-older-keyboard-hidden-after")
        } finally {
            scope.cancel()
            root.deleteRecursively()
        }
    }

    private fun waitForKeyboard(visible: Boolean) {
        compose.waitUntil(5_000) {
            ViewCompat.getRootWindowInsets(compose.activity.window.decorView)?.isVisible(WindowInsetsCompat.Type.ime()) == visible
        }
        var previous = compose.onNodeWithTag("chatMessageInput").fetchSemanticsNode().boundsInRoot
        var stableAt = System.nanoTime()
        compose.waitUntil(5_000) {
            val next = compose.onNodeWithTag("chatMessageInput").fetchSemanticsNode().boundsInRoot
            if (next != previous) { previous = next; stableAt = System.nanoTime() }
            System.nanoTime() - stableAt > 250_000_000
        }
        compose.waitForIdle()
    }

    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val file = File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "$name.png")
        file.parentFile?.mkdirs()
        file.outputStream().use { instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
