package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DirectChatCapabilityState
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class ContactNicknameCardTest {
    @get:Rule val compose = createComposeRule()

    private val nickname = mutableStateOf("")
    private val note = mutableStateOf("")
    private val editing = mutableStateOf(false)
    private val saved = mutableListOf<Pair<String, String>>()
    private var removals = 0

    private fun show(nicknameValue: String? = null, noteValue: String? = null) {
        nickname.value = nicknameValue.orEmpty()
        note.value = noteValue.orEmpty()
        val chat = CurrentChatSnapshot(
            contactIdentity = null, socialConnection = null, chatId = "synthetic-contact",
            kind = ChatKind.DIRECT, displayName = "Test contact", nickname = nicknameValue,
            contactNote = noteValue, profileName = null, subtitle = null, pictureUrl = null,
            about = null, groupId = null, memberCount = 0u, messageTtlSeconds = null,
            isMuted = false, participants = emptyList(), messages = emptyList(),
            typingIndicators = emptyList(), draft = "", isRequest = false,
            directChatCapability = DirectChatCapabilityState.AVAILABLE,
        )
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                Column {
                    ContactNicknameCard(
                        chat = chat,
                        nicknameDraft = nickname.value,
                        onNicknameChange = { nickname.value = it },
                        noteDraft = note.value,
                        onNoteChange = { note.value = it },
                        onSave = { saved += nickname.value to note.value; editing.value = false },
                        onRemove = { removals++; editing.value = false },
                        editing = editing.value,
                        onToggleEditing = { editing.value = !editing.value },
                    )
                }
            }
        }
    }

    private fun openEditor() = compose.onNodeWithTag("directChatNicknameRow").performClick()
    private fun saveButton() = compose.onNodeWithTag("directChatSaveNicknameButton")

    @Test fun opensEmptyContactAndSavesUnicodeDraftWithoutCrashing() {
        show()
        openEditor()
        saveButton().assertIsNotEnabled()
        val draft = "\u0085\u00a0Alice\u2003\u202f😀\u3000"
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement(draft)
        saveButton().assertIsEnabled()
        captureComposerTest("contact-nickname-unicode")
        saveButton().performClick()
        compose.runOnIdle { assertEquals(listOf(draft to ""), saved) }
    }

    @Test fun unicodeWhitespaceFormattingAloneDoesNotEnableSave() {
        show("Alice 😀", "Line one\nLine two")
        openEditor()
        compose.onNodeWithTag("directChatNicknameField")
            .performTextReplacement("\u0085Alice\u00a0\u2003\u202f😀\u0085")
        compose.onNodeWithTag("directChatNoteField")
            .performTextReplacement(" Line one\r\nLine two\r\n ")
        saveButton().assertIsNotEnabled()
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("Changed note")
        saveButton().assertIsEnabled()
        saveButton().performClick()
        compose.runOnIdle { assertEquals("Changed note", saved.single().second) }
    }

    @Test fun codepointLimitsAllowSupplementaryCharactersAtBothBoundaries() {
        show()
        openEditor()
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement("😀".repeat(80))
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("🦄".repeat(240))
        saveButton().assertIsEnabled()
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement("😀".repeat(81))
        saveButton().assertIsNotEnabled()
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement("😀".repeat(80))
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("🦄".repeat(241))
        saveButton().assertIsNotEnabled()
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("e\u0301".repeat(120))
        saveButton().assertIsEnabled()
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("e\u0301".repeat(120) + "x")
        saveButton().assertIsNotEnabled()
    }

    @Test fun unchangedStoredNonWhitespaceControlsDoNotEnableSave() {
        show("\u001cAlice\u001f")
        openEditor()
        saveButton().assertIsNotEnabled()
    }

    @Test fun clearingBothFieldsCanBeSaved() {
        show("Alice", "Private note")
        openEditor()
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement("\u0085\u00a0")
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("")
        saveButton().assertIsEnabled()
        saveButton().performClick()
        compose.runOnIdle { assertEquals(listOf("\u0085\u00a0" to ""), saved) }
    }

    @Test fun cancelDoesNotSaveAndRemovalKeepsItsSeparateAction() {
        show("Alice", "Private note")
        openEditor()
        compose.onNodeWithTag("directChatNicknameField").performTextReplacement("")
        compose.onNodeWithTag("directChatNoteField").performTextReplacement("")
        saveButton().assertIsEnabled()
        compose.onNodeWithTag("directChatCancelNicknameButton").performClick()
        compose.runOnIdle { assertEquals(0, saved.size); assertEquals(0, removals) }
        openEditor()
        compose.onNodeWithTag("directChatRemoveNicknameButton").performClick()
        compose.runOnIdle { assertEquals(0, saved.size); assertEquals(1, removals) }
    }
}
