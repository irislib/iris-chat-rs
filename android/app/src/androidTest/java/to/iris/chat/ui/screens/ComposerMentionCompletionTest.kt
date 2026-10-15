package to.iris.chat.ui.screens

import android.graphics.Bitmap
import java.io.File
import androidx.test.platform.app.InstrumentationRegistry
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.ui.text.TextRange

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.rust.ChatParticipantSnapshot
import to.iris.chat.rust.messageMentions
import to.iris.chat.ui.theme.IrisChatTheme

@OptIn(ExperimentalTestApi::class)
class ComposerMentionCompletionTest {
    @get:Rule val compose = createComposeRule()
    private val first = "11".repeat(32)
    private val second = "22".repeat(32)
    private val people = listOf(first, second).map { ChatParticipantSnapshot(null, it, "Alex", null, false) }

    @Test fun bareAtSelectsExactPersonBeforeSendingAndRestoresIdentity() {
        val draft = TextFieldState("Hello ")
        var sent = 0
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                ComposerBar(draft, emptyList(), false, false, null,
                    onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = { sent++ },
                    mentionParticipants = people)
            }
        }
        val field = compose.onNodeWithTag("chatMessageInput")
        field.performClick().performTextInput("@")
        compose.onNodeWithTag("chatMention-$first").assertIsDisplayed()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
            File(instrumentation.targetContext.cacheDir, "mention-completion.png").outputStream().use {
                bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
            }
            bitmap.recycle()
        }
        compose.onNodeWithTag("chatMention-$second").performClick()
        compose.runOnIdle {
            assertEquals(second, messageMentions(draft.text.toString()).single().ownerPubkeyHex)
            assertEquals(0, sent)
        }
        field.assertIsFocused()
        field.performTextInput("again @")
        field.performKeyInput { pressKey(Key.DirectionDown); pressKey(Key.Enter) }
        compose.runOnIdle {
            assertEquals(listOf(second, second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            assertEquals(0, sent)
        }
        field.performTextInput(":fire")
        field.performTextInput(":")
        compose.runOnIdle {
            assertTrue(draft.text.toString().endsWith("🔥"))
            assertEquals(listOf(second, second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
        }
        compose.onNodeWithTag("chatSendButton").performClick()
        assertEquals(1, sent)
    }
    @OptIn(ExperimentalFoundationApi::class)
    @Test fun equalNamesRemainDistinctThroughVisibleSelectionDeletionAndUndo() {
        val draft = TextFieldState()
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                ComposerBar(draft, emptyList(), false, false, null,
                    onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = {},
                    mentionParticipants = people)
            }
        }
        val field = compose.onNodeWithTag("chatMessageInput")
        field.performClick().performTextInput("@")
        compose.onNodeWithTag("chatMention-$first").performClick()
        compose.runOnIdle {
            assertEquals(listOf(first), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.undo()
            assertEquals("@", draft.text.toString())
            draft.undoState.redo()
            assertEquals(listOf(first), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
        }
        field.performTextInput("@")
        compose.onNodeWithTag("chatMention-$second").performClick()
        // The offsets deliberately address visible labels, not the longer
        // encoded identities. Compose must map them to the correct person.
        field.performTextInputSelection(TextRange(0, 6), relativeToOriginalText = false)
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle {
            assertEquals(listOf(second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.undo()
            assertEquals(listOf(first, second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.redo()
            assertEquals(listOf(second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.undo()
        }
        field.performTextInputSelection(TextRange(6, 12), relativeToOriginalText = false)
        field.performKeyInput { pressKey(Key.Backspace) }
        compose.runOnIdle {
            assertEquals(listOf(first), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.undo()
            assertEquals(listOf(first, second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
        }
        field.performTextInputSelection(TextRange(0, 5), relativeToOriginalText = false)
        field.performTextInput("@Alex")
        compose.runOnIdle {
            assertEquals(listOf(second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
            draft.undoState.undo()
            assertEquals(listOf(first, second), messageMentions(draft.text.toString()).map { it.ownerPubkeyHex })
        }
    }

    @Test fun restoredDraftAndDismissedMenuDoNotReopenWithoutTyping() {
        val draft = TextFieldState("@")
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                ComposerBar(draft, emptyList(), false, false, null,
                    onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = {},
                    mentionParticipants = people)
            }
        }
        val field = compose.onNodeWithTag("chatMessageInput")
        field.performClick()
        compose.onNodeWithTag("chatMentionSuggestions").assertDoesNotExist()
        field.performTextInput("A")
        compose.onNodeWithTag("chatMentionSuggestions").assertIsDisplayed()
        field.performKeyInput { pressKey(Key.Escape) }
        compose.onNodeWithTag("chatMentionSuggestions").assertDoesNotExist()
        compose.runOnIdle { assertEquals("@A", draft.text.toString()) }
    }
}
