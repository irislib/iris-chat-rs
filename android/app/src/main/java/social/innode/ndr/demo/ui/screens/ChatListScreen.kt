package social.innode.ndr.demo.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.rust.ChatKind
import social.innode.ndr.demo.rust.Screen
import social.innode.ndr.demo.rust.proxiedImageUrl
import social.innode.ndr.demo.ui.components.IrisAvatar
import social.innode.ndr.demo.ui.components.IrisChatListRow
import social.innode.ndr.demo.ui.components.IrisDivider
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisPrimaryButton
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.formatRelativeTime
import social.innode.ndr.demo.ui.theme.IrisTheme

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatListScreen(
    appManager: AppManager,
    appState: AppState,
) {
    var relativeNowMillis by remember { mutableStateOf(System.currentTimeMillis()) }
    val account = appState.account

    LaunchedEffect(Unit) {
        while (true) {
            delay(15_000L)
            relativeNowMillis = System.currentTimeMillis()
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Chats",
                leading = {
                    if (account != null) {
                        Box(
                            modifier =
                                Modifier
                                    .padding(start = 4.dp)
                                    .testTag("chatListProfileButton")
                                    .clickable { appManager.pushScreen(Screen.Settings) },
                        ) {
                            IrisAvatar(
                                label = account.displayName,
                                emphasize = true,
                                size = 44.dp,
                                imageUrl =
                                    account.pictureUrl?.let { url ->
                                        proxiedImageUrl(
                                            originalSrc = url,
                                            preferences = appState.preferences,
                                            width = 88u,
                                            height = 88u,
                                            square = true,
                                        )
                                    },
                            )
                        }
                    }
                },
                actions = {
                    IrisPrimaryButton(
                        text = "New",
                        onClick = { appManager.pushScreen(Screen.NewChat) },
                        modifier = Modifier.testTag("chatListNewChatButton"),
                        icon = {
                            Icon(
                                imageVector = IrisIcons.NewChat,
                                contentDescription = null,
                            )
                        },
                    )
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background),
        ) {
            if (appState.chatList.isEmpty()) {
                item {
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 12.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "No chats yet",
                            style = MaterialTheme.typography.bodyLarge,
                            color = IrisTheme.palette.muted,
                        )
                    }
                }
            } else {
                items(appState.chatList, key = { it.chatId }) { chat ->
                    val subtitle = chat.subtitle
                    Column(modifier = Modifier.fillMaxWidth()) {
                        IrisChatListRow(
                            title = chat.displayName,
                            preview =
                                if (chat.isTyping) {
                                    "Typing"
                                } else {
                                    chat.lastMessagePreview ?: subtitle.orEmpty()
                                },
                            timeLabel = formatRelativeTime(chat.lastMessageAtSecs?.toLong(), relativeNowMillis),
                            unreadCount = chat.unreadCount.toLong(),
                            lastMessageMine = chat.lastMessageIsOutgoing == true,
                            lastDelivery = chat.lastMessageDelivery,
                            onClick = { appManager.openChat(chat.chatId) },
                            modifier = Modifier.testTag("chatRow-${chat.chatId.take(12)}"),
                        )
                        if (chat.kind == ChatKind.GROUP && subtitle != null) {
                            Text(
                                text = subtitle,
                                modifier = Modifier.padding(start = 70.dp, bottom = 10.dp),
                                style = MaterialTheme.typography.labelMedium,
                                color = IrisTheme.palette.muted,
                            )
                        }
                        IrisDivider(modifier = Modifier.padding(start = 70.dp))
                    }
                }
            }
        }
    }
}
