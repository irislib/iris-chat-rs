package to.iris.chat.ui.components

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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import to.iris.chat.ui.theme.IrisTheme

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun IrisEmojiPickerSheet(
    onDismiss: () -> Unit,
    onPick: (String) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.background,
        modifier = Modifier.testTag("emojiPickerSheet"),
    ) {
        IrisEmojiPicker(
            onPick = { emoji ->
                onPick(emoji)
                onDismiss()
            },
        )
    }
}

@Composable
fun IrisEmojiPicker(onPick: (String) -> Unit) {
    var query by remember { mutableStateOf("") }
    val palette = IrisTheme.palette
    val trimmed = query.trim()
    val visibleCategories =
        remember(trimmed) {
            if (trimmed.isEmpty()) {
                IrisEmojiCatalog
            } else {
                IrisEmojiCatalog.mapNotNull { (name, emojis) ->
                    val nameMatches = name.contains(trimmed, ignoreCase = true)
                    val hits = if (nameMatches) emojis else emojis.filter { it.contains(trimmed) }
                    if (hits.isEmpty()) null else name to hits
                }
            }
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
                items(emojis, key = { "$name-$it" }) { emoji ->
                    Box(
                        modifier =
                            Modifier
                                .size(44.dp)
                                .clickable { onPick(emoji) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(text = emoji, fontSize = 26.sp)
                    }
                }
            }
        }
    }
}

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
