package to.iris.chat.ui.screens

import android.content.Context
import android.content.ContextWrapper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.AnnotatedString
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
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

/** Draft lifecycle through the production ChatScreen and native text editor. */
class ChatDraftPersistenceTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test fun externalRestorePreservesNewerLocalTextAndDebouncesOneSave() = withChat { fixture ->
        val input = compose.onNodeWithTag("chatMessageInput")
        assertTrue(fixture.saves().isEmpty())
        fixture.restore("Restored elsewhere")
        input.assertTextEquals("Restored elsewhere")

        input.performTextReplacement("A new local draft")
        compose.mainClock.advanceTimeByFrame()
        fixture.restore("A stale remote draft")
        input.assertTextEquals("A new local draft")
        compose.mainClock.advanceTimeBy(300)
        compose.waitForIdle()
        assertTrue("An unfinished debounce must not save", fixture.saves().isEmpty())
        compose.mainClock.advanceTimeBy(300)
        compose.waitForIdle()
        assertEquals(listOf("A new local draft"), fixture.saves())

        // An acknowledgement must not replace the editor or emit another save.
        fixture.restore("A new local draft")
        compose.mainClock.advanceTimeBy(600)
        compose.waitForIdle()
        input.assertTextEquals("A new local draft")
        assertEquals(listOf("A new local draft"), fixture.saves())
    }

    @Test fun sendUsesCurrentTextAndLeavingFlushesTheNextDraftAndStopsTyping() = withChat { fixture ->
        val input = compose.onNodeWithTag("chatMessageInput")
        input.performTextReplacement("Send the current text")
        compose.onNodeWithTag("chatSendButton").performClick()
        compose.mainClock.advanceTimeBy(600)
        compose.waitForIdle()
        assertEquals(listOf("Send the current text"), fixture.rust.dispatchedActions
            .filterIsInstance<AppAction.SendMessage>().map { it.text })
        input.assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString("")))
        assertEquals(listOf(""), fixture.saves())

        input.performTextInput("Keep this on leaving")
        compose.mainClock.advanceTimeByFrame()
        compose.waitForIdle()
        assertTrue(fixture.rust.dispatchedActions.any { it is AppAction.SendTyping && it.chatId == fixture.chatId })
        val stopsBeforeLeaving = fixture.rust.dispatchedActions.filterIsInstance<AppAction.StopTyping>().size
        fixture.leave()
        compose.onNodeWithTag("chatMessageInput").assertDoesNotExist()
        assertEquals(listOf("", "Keep this on leaving"), fixture.saves())
        assertEquals(stopsBeforeLeaving + 1, fixture.rust.dispatchedActions.filterIsInstance<AppAction.StopTyping>().size)
        compose.mainClock.advanceTimeBy(600)
        compose.waitForIdle()
        assertEquals("Disposed debounce must not repeat its flush", listOf("", "Keep this on leaving"), fixture.saves())
    }

    @Test fun editSaveAndCancelPreserveTheUnsentDraft() = withChat { fixture ->
        val input = compose.onNodeWithTag("chatMessageInput")
        compose.onNodeWithTag("chatMessage-edit-target").performTouchInput { longClick() }
        compose.onNodeWithText("Edit", useUnmergedTree = true).performClick()
        input.assertTextEquals("Original message")
        input.performTextReplacement("Corrected message")
        compose.mainClock.advanceTimeBy(600)
        compose.onNodeWithTag("chatSendButton").performClick()
        input.assertTextEquals("Saved caption")
        assertEquals(listOf("Corrected message"), fixture.rust.dispatchedActions
            .filterIsInstance<AppAction.EditMessage>().map { it.text })
        assertTrue("Editing must not replace the saved draft", fixture.saves().isEmpty())
        assertTrue(fixture.rust.dispatchedActions.none { it is AppAction.SendMessage })

        compose.onNodeWithTag("chatMessage-edit-target").performTouchInput { longClick() }
        compose.onNodeWithText("Edit", useUnmergedTree = true).performClick()
        input.performTextReplacement("Discard this edit")
        compose.onNodeWithText("Cancel").performClick()
        input.assertTextEquals("Saved caption")
        assertEquals(1, fixture.rust.dispatchedActions.filterIsInstance<AppAction.EditMessage>().size)
    }

    @Test fun deleteForEveryoneRequiresConfirmationAndKeepsLocalDeleteSeparate() = withChat { fixture ->
        compose.onNodeWithTag("chatMessage-edit-target").performTouchInput { longClick() }
        compose.onNodeWithText("Delete for everyone", useUnmergedTree = true).performClick()
        assertTrue(fixture.rust.dispatchedActions.none { it is AppAction.DeleteMessageForEveryone })
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithTag("chatMessage-edit-target").performTouchInput { longClick() }
        compose.onNodeWithText("Delete for everyone", useUnmergedTree = true).performClick()
        compose.onNodeWithText("Delete for everyone", useUnmergedTree = true).performClick()
        assertEquals(listOf("edit-target"), fixture.rust.dispatchedActions
            .filterIsInstance<AppAction.DeleteMessageForEveryone>().map { it.messageId })
        assertTrue(fixture.rust.dispatchedActions.none { it is AppAction.DeleteLocalMessage })
    }

    @Test fun editedLabelShowsVersionsAndDeletedMessageHidesActions() = withChat { fixture ->
        fixture.updateMessage { message -> message.copy(body = "Corrected message", editHistory = listOf(
            MessageEditSnapshot("original", "Original message", 1uL),
            MessageEditSnapshot("edited", "Corrected message", 2uL),
        )) }
        compose.onNodeWithTag("chatMessageEdited-edit-target").performClick()
        compose.onNodeWithTag("messageEditHistory").assertExists()
        compose.onNodeWithText("Original").assertExists()
        compose.onNodeWithText("Current").assertExists()
        compose.onNodeWithText("Close").performClick()
        fixture.updateMessage { it.copy(body = "", editHistory = emptyList(), deletedForEveryone = true) }
        compose.onNodeWithText("Message deleted").assertExists()
        compose.onNodeWithTag("chatMessageEdited-edit-target").assertDoesNotExist()
        compose.onNodeWithTag("chatMessage-edit-target").performTouchInput { longClick() }
        compose.onNodeWithText("Edit", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Forward", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Delete for everyone", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Delete for me", useUnmergedTree = true).assertExists()
    }

    private fun withChat(block: (Fixture) -> Unit) {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(base.cacheDir, "draft-${UUID.randomUUID()}").apply { mkdirs() }
        val context = object : ContextWrapper(base) {
            override fun getApplicationContext(): Context = this
            override fun getFilesDir() = File(root, "files").apply { mkdirs() }
            override fun getCacheDir() = File(root, "cache").apply { mkdirs() }
        }
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val state = buildLargeTestAppState(1u, 0u, 1u)
        val chat = state.currentChat!!.copy(directChatCapability = DirectChatCapabilityState.AVAILABLE,
            isRequest = false, messageTtlSeconds = null, draft = "Saved caption",
            messages = state.currentChat!!.messages.map { it.copy(id = "edit-target", kind = ChatMessageKind.USER, body = "Original message",
                isOutgoing = true, delivery = DeliveryState.SENT, attachments = emptyList(), call = null) })
        val rust = MockRustAppClient(state.copy(currentChat = chat,
            router = Router(Screen.ChatList, listOf(Screen.Chat(chat.chatId)))))
        val store = PreferenceDataStoreFactory.create(scope = scope) { File(root, "test.preferences_pb") }
        val manager = AppManager(context, scope, RecordingSecureSecretStore(), dataStore = store,
            rustFactory = { _, _ -> rust }, clearDeliveredNotifications = {})
        val visible = mutableStateOf(true)
        val fixture = Fixture(rust, chat.chatId) {
            compose.runOnIdle { visible.value = false }
            compose.mainClock.advanceTimeByFrame()
            compose.waitForIdle()
        }
        try {
            compose.mainClock.autoAdvance = false
            compose.setContent { IrisChatTheme(darkTheme = false) {
                if (visible.value) ChatScreen(manager, chat.chatId)
            } }
            compose.mainClock.advanceTimeBy(64)
            compose.onNodeWithTag("chatMessageInput").assertTextEquals("Saved caption").performClick()
            block(fixture)
        } finally {
            fixture.leave()
            scope.cancel()
            root.deleteRecursively()
        }
    }

    private inner class Fixture(val rust: MockRustAppClient, val chatId: String, val leave: () -> Unit) {
        fun saves() = rust.dispatchedActions.filterIsInstance<AppAction.SetChatDraft>()
            .filter { it.chatId == chatId }.map { it.text }

        fun updateMessage(transform: (ChatMessageSnapshot) -> ChatMessageSnapshot) {
            compose.runOnIdle {
                val state = rust.currentState
                val updated = state.copy(rev = state.rev + 1uL,
                    currentChat = state.currentChat!!.copy(messages = state.currentChat!!.messages.map(transform)))
                rust.currentState = updated
                rust.emit(AppUpdate.FullState(updated))
            }
            compose.mainClock.advanceTimeBy(64)
            compose.waitForIdle()
        }

        fun restore(text: String) {
            compose.runOnIdle {
                val updated = rust.currentState.copy(rev = rust.currentState.rev + 1uL,
                    currentChat = rust.currentState.currentChat!!.copy(draft = text))
                rust.currentState = updated
                rust.emit(AppUpdate.FullState(updated))
            }
            compose.mainClock.advanceTimeBy(64)
            compose.waitForIdle()
        }
    }
}
