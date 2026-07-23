package to.iris.chat.ui.screens

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.FollowedUserSearchResult
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.formatRelativeTime
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun SearchSectionHeader(title: String) {
    Text(
        text = title,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 4.dp),
        style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold),
        color = IrisTheme.palette.muted,
    )
}

@Composable
internal fun SearchChatRow(
    appManager: AppManager,
    appState: AppState,
    chat: ChatThreadSnapshot,
) {
    val avatarData by rememberNhashImageData(appManager, chat.pictureUrl)
    IrisChatListRow(
        title = chat.displayName,
        isMuted = chat.isMuted,
        isPinned = chat.isPinned,
        preview = chat.chatListPreview(),
        timeLabel = formatRelativeTime(chat.lastMessageAtSecs?.toLong(), System.currentTimeMillis()),
        imageUrl = proxiedAvatarUrl(chat.pictureUrl, appState),
        imageData = avatarData,
        unreadCount = chat.unreadCount.toLong(),
        lastMessageMine = chat.lastMessageIsOutgoing == true,
        lastDelivery = chat.lastMessageDelivery,
        onClick = { appManager.openChat(chat.chatId) },
    )
}

@Composable
internal fun FollowedPersonSearchRow(
    appManager: AppManager,
    appState: AppState,
    person: FollowedUserSearchResult,
) {
    val avatarData by rememberNhashImageData(appManager, person.pictureUrl)
    val preview = person.profileLabel
        ?.takeIf { !it.equals(person.displayLabel, ignoreCase = true) }
        ?: person.about
        ?: person.userId
    IrisChatListRow(
        title = person.displayLabel,
        preview = preview,
        timeLabel = null,
        imageUrl = proxiedAvatarUrl(person.pictureUrl, appState),
        imageData = avatarData,
        unreadCount = 0,
        lastMessageMine = false,
        lastDelivery = null,
        onClick = { appManager.dispatch(AppAction.CreateChat(person.ownerPubkeyHex)) },
        modifier = Modifier.testTag("personRow-${person.ownerPubkeyHex.take(12)}"),
    )
}

private fun proxiedAvatarUrl(pictureUrl: String?, appState: AppState): String? =
    pictureUrl
        ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
        ?.let { url ->
            proxiedImageUrl(
                originalSrc = url,
                preferences = appState.preferences,
                width = 84u,
                height = 84u,
                square = true,
            )
        }
