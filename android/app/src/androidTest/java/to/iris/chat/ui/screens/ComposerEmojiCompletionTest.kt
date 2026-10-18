package to.iris.chat.ui.screens

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import to.iris.chat.ui.theme.IrisChatTheme

class ComposerEmojiCompletionTest {
    @get:Rule val compose = createComposeRule()

    @Test fun typingColonKeepsMobileDraftLiteralAndDoesNotOfferEmoji() {
        val draft = TextFieldState("Hello ")
        var sent = 0
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                ComposerTestFrame {
                    ComposerBar(draft, emptyList(), false, false, null,
                        onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = { sent++ })
                }
            }
        }
        val field = compose.onNodeWithTag("chatMessageInput")
        field.performClick().performTextInput(":sm")
        compose.onNodeWithTag("chatEmojiCompletion-smile").assertDoesNotExist()
        captureComposerTest("emoji-completion")
        field.performTextInput("ile:")
        compose.runOnIdle {
            assertEquals("Hello :smile:", draft.text.toString())
            assertEquals(0, sent)
        }
        field.assertIsFocused()
        compose.onNodeWithTag("chatSendButton").performClick()
        assertEquals(1, sent)
    }

    @Test fun restoredDraftAndClosingColonRemainLiteral() {
        val draft = TextFieldState(":sm")
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                ComposerBar(draft, emptyList(), false, false, null,
                    onDraftChange = {}, onAttach = {}, onRemoveAttachment = {}, onSend = {})
            }
        }
        val field = compose.onNodeWithTag("chatMessageInput")
        field.performClick()
        compose.onNodeWithTag("chatEmojiCompletion-smile").assertDoesNotExist()
        field.performTextClearance()
        field.performTextInput(":fire")
        field.performTextInput(":")
        compose.runOnIdle { assertEquals(":fire:", draft.text.toString()) }
        field.performTextClearance()
        field.performTextInput("https://iris.to/:sm 12:30")
        compose.runOnIdle { assertEquals("https://iris.to/:sm 12:30", draft.text.toString()) }
        compose.onNodeWithTag("chatEmojiCompletion-smile").assertDoesNotExist()
    }
}
