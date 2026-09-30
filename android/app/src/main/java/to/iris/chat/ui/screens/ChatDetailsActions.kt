package to.iris.chat.ui.screens

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.filled.Search
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.ui.components.IrisMenuRow

@Composable
internal fun ChatDetailsActions(appManager: AppManager, chatId: String, name: String) {
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val pinned = chatId in preferences.pinnedChatIds
    var searchOpen by remember(chatId) { mutableStateOf(false) }
    IrisMenuRow(
        title = "Search in chat", icon = Icons.Filled.Search,
        onClick = { searchOpen = true }, modifier = Modifier.testTag("chatDetailsSearchButton"),
    )
    IrisMenuRow(
        title = if (pinned) "Unpin chat" else "Pin chat", icon = Icons.Filled.PushPin,
        onClick = { appManager.dispatch(AppAction.SetChatPinned(chatId, !pinned)) },
        modifier = Modifier.testTag("chatDetailsPinButton"),
    )
    if (searchOpen) InChatSearchSheet(appManager, chatId, name, onDismiss = { searchOpen = false })
}
