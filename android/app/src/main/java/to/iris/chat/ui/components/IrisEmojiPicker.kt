package to.iris.chat.ui.components

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import to.iris.chat.ui.theme.IrisTheme
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun IrisEmojiPickerSheet(
    onDismiss: () -> Unit,
    onPick: (String) -> Unit,
    suggestedEmojis: List<String> = emptyList(),
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.background,
        modifier = Modifier.testTag("emojiPickerSheet"),
    ) {
        IrisEmojiPicker(
            suggestedEmojis = suggestedEmojis,
            onPick = { emoji ->
                onPick(emoji)
                onDismiss()
            },
        )
    }
}

@Composable
fun IrisEmojiPicker(
    onPick: (String) -> Unit,
    suggestedEmojis: List<String> = emptyList(),
) {
    var query by remember { mutableStateOf("") }
    val palette = IrisTheme.palette
    val context = LocalContext.current.applicationContext
    var recentEmojis by remember { mutableStateOf(loadRecentReactionEmojis(context)) }
    val trimmed = query.trim()
    val visibleCategories =
        remember(trimmed, suggestedEmojis, recentEmojis) {
            if (trimmed.isEmpty()) {
                val postEmojis = uniqueReactionEmojis(suggestedEmojis)
                val recent = uniqueReactionEmojis(recentEmojis).filterNot { it in postEmojis }
                buildList {
                    if (postEmojis.isNotEmpty()) add("On this post" to postEmojis)
                    if (recent.isNotEmpty()) add("Recent" to recent)
                    addAll(IrisEmojiCatalog)
                }
            } else {
                IrisEmojiCatalog.mapNotNull { (name, emojis) ->
                    val hits = emojis.filter { irisEmojiMatchesQuery(it, name, trimmed) }
                    if (hits.isEmpty()) null else name to hits
                }
            }
        }

    fun pick(emoji: String) {
        recentEmojis = rememberRecentReactionEmoji(context, emoji)
        onPick(emoji)
    }

    Column(modifier = Modifier.fillMaxWidth().heightIn(min = 360.dp, max = 540.dp)) {
        TextField(
            value = query,
            onValueChange = { query = it },
            placeholder = {
                Text("Search", color = palette.muted)
            },
            leadingIcon = {
                Icon(
                    imageVector = Icons.Rounded.Search,
                    contentDescription = null,
                    tint = palette.muted,
                )
            },
            singleLine = true,
            keyboardOptions =
                KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 6.dp)
                    .testTag("emojiPickerSearch"),
            colors =
                TextFieldDefaults.colors(
                    focusedContainerColor = palette.panel,
                    unfocusedContainerColor = palette.panel,
                    disabledContainerColor = palette.panel,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    disabledIndicatorColor = Color.Transparent,
                ),
            shape = RoundedCornerShape(12.dp),
        )

        LazyVerticalGrid(
            columns = GridCells.Adaptive(minSize = 44.dp),
            contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            visibleCategories.forEach { (name, emojis) ->
                item(span = { GridItemSpan(maxLineSpan) }, key = "header-$name") {
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .background(MaterialTheme.colorScheme.background)
                                .padding(top = 10.dp, bottom = 4.dp, start = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = name,
                            style = MaterialTheme.typography.labelMedium,
                            color = palette.muted,
                        )
                    }
                }
                items(emojis.size, key = { index -> "$name-$index-${emojis[index]}" }) { index ->
                    val emoji = emojis[index]
                    Box(
                        modifier =
                            Modifier
                                .size(44.dp)
                                .clickable { pick(emoji) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(text = emoji, fontSize = 26.sp)
                    }
                }
            }
        }
    }
}

internal val IrisDefaultReactionEmojis = listOf("❤️", "👍", "😂", "😮", "😢", "🙏", "🔥")

private const val RecentReactionEmojiPrefsName = "iris_emoji_picker"
private const val RecentReactionEmojiKey = "recent_reactions"
private const val RecentReactionEmojiLimit = 16

internal fun loadRecentReactionEmojis(context: Context): List<String> {
    val raw =
        context
            .getSharedPreferences(RecentReactionEmojiPrefsName, Context.MODE_PRIVATE)
            .getString(RecentReactionEmojiKey, "")
            .orEmpty()
    if (raw.isBlank()) return emptyList()
    return uniqueReactionEmojis(raw.split("\n"))
}

internal fun rememberRecentReactionEmoji(
    context: Context,
    emoji: String,
): List<String> {
    val trimmed = emoji.trim()
    if (trimmed.isEmpty()) return loadRecentReactionEmojis(context)
    val values =
        uniqueReactionEmojis(
            listOf(trimmed) + loadRecentReactionEmojis(context).filterNot { it == trimmed },
        ).take(RecentReactionEmojiLimit)
    context
        .getSharedPreferences(RecentReactionEmojiPrefsName, Context.MODE_PRIVATE)
        .edit()
        .putString(RecentReactionEmojiKey, values.joinToString("\n"))
        .apply()
    return values
}

internal fun irisReactionQuickChoices(
    postSuggestions: List<String>,
    recentEmojis: List<String>,
): List<String> =
    uniqueReactionEmojis(postSuggestions + recentEmojis + IrisDefaultReactionEmojis).take(7)

internal fun uniqueReactionEmojis(emojis: List<String>): List<String> {
    val seen = linkedSetOf<String>()
    emojis.forEach { emoji ->
        val trimmed = emoji.trim()
        if (trimmed.isNotEmpty()) seen += trimmed
    }
    return seen.toList()
}

internal fun irisEmojiMatchesQuery(
    emoji: String,
    category: String,
    query: String,
): Boolean {
    val tokens = normalizeEmojiSearchText(query).split(" ").filter { it.isNotBlank() }
    if (tokens.isEmpty()) return true
    val names =
        emoji
            .codePoints()
            .toArray()
            .map { codePoint -> runCatching { Character.getName(codePoint) }.getOrNull() }
            .filterNotNull()
            .joinToString(" ")
    val aliases = IrisEmojiSearchAliases[emoji].orEmpty()
    val haystack = normalizeEmojiSearchText("$emoji $category $names $aliases")
    return tokens.all { haystack.contains(it) }
}

private fun normalizeEmojiSearchText(value: String): String =
    value
        .lowercase(Locale.ROOT)
        .replace('_', ' ')
        .replace('-', ' ')

private val IrisEmojiSearchAliases =
    mapOf(
        "😂" to "laugh laughing lol haha",
        "🤣" to "laugh laughing lol haha rolling",
        "😊" to "smile smiling happy",
        "🙂" to "smile smiling happy",
        "😍" to "love heart eyes",
        "🥰" to "love hearts",
        "😘" to "kiss love",
        "😢" to "sad tear crying",
        "😭" to "sad cry crying",
        "😠" to "angry mad",
        "🤬" to "angry mad swearing",
        "🙏" to "pray praying thanks thank you please",
        "👏" to "clap applause",
        "🙌" to "hooray yay hands",
        "❤️" to "love heart red",
        "♥️" to "love heart red",
        "🔥" to "fire lit hot",
        "🎉" to "party celebrate celebration",
        "🎊" to "party celebrate celebration",
        "✨" to "sparkle sparkles",
        "✅" to "yes check done",
        "❌" to "no cross x",
        "👀" to "eyes look watching",
        "💯" to "hundred perfect",
    )

internal val IrisEmojiCatalog: List<Pair<String, List<String>>> =
    listOf(
        "Smileys" to
            listOf(
                "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "🙂",
                "🙃", "😉", "😍", "🥰", "😘", "😎", "🤩", "🥳", "😏", "😌",
                "😴", "😪", "🤤", "😋", "😜", "🤪", "😝", "🤔", "🤨", "😐",
                "😑", "😶", "🙄", "😬", "🤐", "🤧", "🤒", "🤕", "😇", "🤠",
                "🥺", "😢", "😭", "😠", "🤬", "🤯", "🥶", "🥵", "😱", "😨",
                "😰", "😳", "🤗",
            ),
        "Hearts" to
            listOf(
                "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💖",
                "💗", "💓", "💞", "💕", "💘", "💝", "💟", "♥️", "💔", "❣️",
                "❤️‍🔥", "❤️‍🩹",
            ),
        "Hands" to
            listOf(
                "👍", "👎", "👌", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉",
                "👆", "👇", "☝️", "✋", "🤚", "🖐", "🖖", "👋", "🤝", "🙏",
                "👏", "🙌", "💪", "🫶", "🫰", "🫵", "🫱", "🫲",
            ),
        "Animals" to
            listOf(
                "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯",
                "🦁", "🐮", "🐷", "🐸", "🐵", "🙈", "🙉", "🙊", "🐔", "🐧",
                "🐦", "🦅", "🦉", "🦄", "🐝", "🦋", "🐞", "🐢", "🐍", "🦖",
                "🐙", "🦀", "🐬", "🐳", "🦈",
            ),
        "Food" to
            listOf(
                "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🫐",
                "🍒", "🍑", "🥭", "🍍", "🥥", "🥝", "🍅", "🥑", "🥕", "🌽",
                "🍆", "🥔", "🍕", "🍔", "🍟", "🌭", "🍿", "🥪", "🌮", "🌯",
                "🍣", "🍜", "🍝", "🍦", "🍩", "🍪", "🎂", "🍰", "☕", "🍵",
                "🍺", "🥂", "🍷", "🥃",
            ),
        "Activities" to
            listOf(
                "⚽", "🏀", "🏈", "⚾", "🥎", "🎾", "🏐", "🏉", "🎱", "🪀",
                "🏓", "🏸", "🥅", "🏒", "🏑", "🥍", "🏏", "🪃", "🥊", "🥋",
                "🎽", "⛸", "🥌", "🛷", "🪂", "🏋️", "🤸", "🤺", "🏇", "⛷",
                "🏂", "🏌️", "🏄", "🚣", "🏊", "🤽", "🚴", "🚵", "🎯", "🎮",
                "🎲", "🎼", "🎤", "🎧", "🎷", "🎸", "🥁",
            ),
        "Travel" to
            listOf(
                "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐",
                "🛻", "🚚", "🚛", "🚜", "🛵", "🏍", "🛺", "🚲", "🛴", "🛹",
                "🚂", "✈️", "🚀", "🛸", "🛶", "⛵", "🚢", "🚁", "🗺", "🗽",
                "🗼", "🏰", "🎡", "🎢", "🎠", "🏖", "🏝", "🏔", "🌋", "🏕",
                "🌄", "🌅", "🌌",
            ),
        "Objects" to
            listOf(
                "📱", "💻", "⌨️", "🖥", "🖨", "🖱", "💾", "💿", "📷", "📸",
                "📹", "🎥", "📺", "📻", "📞", "☎️", "🔌", "🔋", "💡", "🔦",
                "🕯", "🧯", "🛢", "💵", "💰", "💳", "💎", "⚖️", "🔧", "🔨",
                "🛠", "⛏", "🪛", "🪚", "🔩", "⚙️", "🧱", "⛓", "🧲", "🔫",
                "💣", "🧨",
            ),
        "Symbols" to
            listOf(
                "✅", "❎", "✔️", "❌", "⭕", "🚫", "⚠️", "🔱", "☑️", "💯",
                "🔥", "✨", "🌟", "⭐", "🌈", "☀️", "🌙", "⚡", "☄️", "💥",
                "🌊", "💧", "💦", "🎉", "🎊", "🎁", "🎀", "🎈", "🪅", "🍾",
                "🥇", "🥈", "🥉", "🏆", "🎖", "🏅", "💤", "💭", "🗯", "💬",
                "🆗", "🆕", "🆒", "🆓", "🆙", "🔝", "♻️", "☮️", "✝️", "☪️",
                "🕉", "☸️", "✡️", "☯️", "☦️",
            ),
    )
