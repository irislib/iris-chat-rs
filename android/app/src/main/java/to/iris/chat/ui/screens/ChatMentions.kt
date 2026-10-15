package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.*
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.input.OutputTransformation
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import to.iris.chat.rust.ChatParticipantSnapshot
import to.iris.chat.rust.formatMessageMention
import to.iris.chat.rust.messageMentions

// Keep canonical identities in TextFieldState (and saved drafts), and let
// Compose map cursor/selection offsets through the visible name replacements.
internal val mentionOutputTransformation = OutputTransformation {
    messageMentions(asCharSequence().toString()).asReversed().forEach { mention ->
        val start = mention.start.toInt()
        replace(start, start + mention.length.toInt(), mention.label)
    }
}

internal data class MentionQuery(val start: Int, val end: Int, val query: String)

internal fun mentionQuery(text: String, selection: TextRange): MentionQuery? {
    if (!selection.collapsed || selection.start !in 0..text.length) return null
    val caret = selection.start
    val start = text.lastIndexOf('@', caret - 1)
    if (start < 0 || (start > 0 && !text[start - 1].isWhitespace())) return null
    val query = text.substring(start + 1, caret)
    if (query.length > 64 || query.any { !it.isLetterOrDigit() && it !in " _-" }) return null
    var end = caret
    while (end < text.length && (text[end].isLetterOrDigit() || text[end] in "_-")) {
        end++
        if (end - start > 65) return null
    }
    return MentionQuery(start, end, query)
}

internal class ChatMentionCompletion(private val draft: TextFieldState) {
    var participants: List<ChatParticipantSnapshot> = emptyList()
    private var activeText by mutableStateOf<String?>(null)
    private var dismissedText by mutableStateOf<String?>(null)
    var focused by mutableStateOf(false)
    var selectedIndex by mutableIntStateOf(0)
        private set
    private val token: MentionQuery?
        get() = if (!focused || draft.composition != null || draft.text.toString() != activeText || activeText == dismissedText) null
            else mentionQuery(draft.text.toString(), draft.selection)
    val results: List<ChatParticipantSnapshot>
        get() = token?.let { query -> participants.filter { it.displayName.contains(query.query, ignoreCase = true) }.take(6) }.orEmpty()
    val inputTransformation = InputTransformation {
        activeText = asCharSequence().toString()
        dismissedText = null
        selectedIndex = 0
    }
    fun dismiss() { dismissedText = activeText }
    fun insert(person: ChatParticipantSnapshot) {
        val candidate = token ?: return
        val text = draft.text.toString()
        val mention = formatMessageMention(person.ownerPubkeyHex, person.displayName)
        if (mention.isEmpty()) return
        val space = if (candidate.end == text.length || !text[candidate.end].isWhitespace()) " " else ""
        draft.edit {
            replace(candidate.start, candidate.end, mention + space)
            selection = TextRange(candidate.start + mention.length + space.length)
        }
        dismiss()
    }
    fun onKeyEvent(event: KeyEvent): Boolean {
        if (event.type != KeyEventType.KeyDown) return false
        if (event.isAltPressed || event.isCtrlPressed || event.isMetaPressed || event.isShiftPressed) {
            if (event.key == Key.Z) dismiss()
            return false
        }
        val choices = results
        if (choices.isEmpty()) return false
        when (event.key) {
            Key.DirectionUp -> selectedIndex = (selectedIndex + choices.size - 1) % choices.size
            Key.DirectionDown -> selectedIndex = (selectedIndex + 1) % choices.size
            Key.Enter, Key.Tab -> insert(choices[selectedIndex.coerceIn(choices.indices)])
            Key.Escape -> dismiss()
            Key.DirectionLeft, Key.DirectionRight -> { dismiss(); return false }
            else -> return false
        }
        return true
    }
}

@Composable
internal fun rememberChatMentionCompletion(draft: TextFieldState, participants: List<ChatParticipantSnapshot>): ChatMentionCompletion {
    val completion = remember(draft) { ChatMentionCompletion(draft) }
    completion.participants = participants
    LaunchedEffect(draft) {
        var previousText = draft.text.toString()
        var previousSelection = draft.selection
        snapshotFlow { draft.text.toString() to draft.selection }.collect { (text, selection) ->
            if (text == previousText && selection != previousSelection) completion.dismiss()
            previousText = text
            previousSelection = selection
        }
    }
    return completion
}

@Composable
internal fun ChatMentionSuggestions(completion: ChatMentionCompletion) {
    val results = completion.results
    if (results.isEmpty()) return
    Column(Modifier.fillMaxWidth().testTag("chatMentionSuggestions")) {
        results.forEachIndexed { index, person ->
            TextButton(onClick = { completion.insert(person) },
                modifier = Modifier.fillMaxWidth().focusProperties { canFocus = false }
                    .background(if (index == completion.selectedIndex) MaterialTheme.colorScheme.primary.copy(alpha = 0.12f) else Color.Transparent)
                    .testTag("chatMention-${person.ownerPubkeyHex}")) {
                Column(Modifier.fillMaxWidth()) {
                    Text(person.displayName, style = MaterialTheme.typography.bodyMedium)
                    if (results.count { it.displayName == person.displayName } > 1) {
                        Text(person.ownerPubkeyHex.take(8), style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
        }
    }
}
