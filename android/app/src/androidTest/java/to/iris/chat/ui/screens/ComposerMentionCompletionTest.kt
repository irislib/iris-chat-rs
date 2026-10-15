package to.iris.chat.ui.screens

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
        compose.onNodeWithTag("chatSendButton").performClick()
        assertEquals(1, sent)
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
