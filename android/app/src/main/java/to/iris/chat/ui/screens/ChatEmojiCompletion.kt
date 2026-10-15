package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import to.iris.chat.rust.irisEmojiSearchAliases
import to.iris.chat.ui.theme.IrisTheme

internal class ChatEmojiCompletion(private val draft: TextFieldState, private val entries: List<ComposerEmojiEntry>) {
    private var activeText by mutableStateOf<String?>(null)
    private var dismissedText by mutableStateOf<String?>(null)
    var focused by mutableStateOf(false)
    var selectedIndex by mutableIntStateOf(0)
        private set

    private val token: ComposerEmojiToken?
        get() = if (!focused || draft.composition != null || draft.text.toString() != activeText || activeText == dismissedText) null
            else ComposerEmojiToken.find(draft.text.toString(), draft.selection.start, draft.selection.end)

    val results: List<ComposerEmojiEntry>
        get() = token?.takeUnless { it.closed }?.let { ComposerEmojiEntry.search(it.query, entries) }.orEmpty()

    // InputTransformation runs only for user edits, so restored drafts and undo
    // do not reopen the menu or rewrite literal shortcodes.
    val inputTransformation = InputTransformation {
        val text = asCharSequence().toString()
        val candidate = ComposerEmojiToken.find(text, selection.start, selection.end)
        if (draft.composition == null && candidate?.closed == true && text.length == originalText.length + 1 && text.getOrNull(selection.start - 1) == ':') {
            entries.firstOrNull { it.matchesExactly(candidate.query) }?.let {
                replace(candidate.start, candidate.end, it.emoji)
                selection = TextRange(candidate.start + it.emoji.length)
            }
        }
        activeText = asCharSequence().toString()
        dismissedText = null
        selectedIndex = 0
    }

    fun dismiss() { dismissedText = activeText }

    fun insert(entry: ComposerEmojiEntry) {
        val candidate = token ?: return
        val text = draft.text.toString()
        val space = if (candidate.end == text.length || !text[candidate.end].isWhitespace()) " " else ""
        draft.edit {
            replace(candidate.start, candidate.end, entry.emoji + space)
            selection = TextRange(candidate.start + entry.emoji.length + space.length)
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
internal fun rememberChatEmojiCompletion(draft: TextFieldState): ChatEmojiCompletion {
    val completion = remember(draft) {
        ChatEmojiCompletion(draft, irisEmojiSearchAliases().mapNotNull {
            val names = it.keywords.split(' ').filter(String::isNotEmpty)
            if (names.isEmpty()) null else ComposerEmojiEntry(it.emoji, names)
        })
    }
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
internal fun ChatEmojiCompletionSuggestions(completion: ChatEmojiCompletion) {
    val choices = completion.results
    if (choices.isEmpty()) return
    Column(Modifier.fillMaxWidth().background(IrisTheme.palette.panelAlt, RoundedCornerShape(12.dp)).padding(4.dp).testTag("chatEmojiSuggestions")) {
        choices.forEachIndexed { index, entry ->
            Row(Modifier.fillMaxWidth()
                .background(if (index == completion.selectedIndex) IrisTheme.palette.accent.copy(alpha = 0.14f) else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(8.dp))
                .focusProperties { canFocus = false }
                .clickable { completion.insert(entry) }
                .padding(horizontal = 10.dp, vertical = 6.dp)
                .testTag("chatEmojiCompletion-${entry.label}")) {
                Text(entry.emoji, fontSize = 24.sp, modifier = Modifier.width(36.dp))
                Text(":${entry.label}:", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 5.dp), maxLines = 1)
            }
        }
    }
}
