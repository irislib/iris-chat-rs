package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.Screen
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.IrisDivider
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.formatRelativeTime
import to.iris.chat.ui.theme.IrisTheme

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatListScreen(
    appManager: AppManager,
    appState: AppState,
    nearbyService: IrisNearbyService? = null,
    onNearbyClick: () -> Unit = {},
) {
    var relativeNowMillis by remember { mutableStateOf(System.currentTimeMillis()) }
    var nearbyTick by remember { mutableStateOf(0) }
    val account = appState.account

    LaunchedEffect(Unit) {
        while (true) {
            delay(15_000L)
            relativeNowMillis = System.currentTimeMillis()
        }
    }

    LaunchedEffect(nearbyService) {
        while (nearbyService != null) {
            delay(1_000L)
            nearbyTick += 1
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            IrisTopBar(
                title = "Chats",
                leading = {
                    if (account != null) {
                        val accountAvatarBytes by rememberNhashImageData(appManager, account.pictureUrl)
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
                                    account.pictureUrl
                                        ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
                                        ?.let { url ->
                                            proxiedImageUrl(
                                                originalSrc = url,
                                                preferences = appState.preferences,
                                                width = 88u,
                                                height = 88u,
                                                square = true,
                                            )
                                        },
                                imageData = accountAvatarBytes,
                            )
                        }
                    }
                },
                actions = {
                    Box(
                        modifier =
                            Modifier
                                .padding(end = 4.dp)
                                .size(40.dp)
                                .background(IrisTheme.palette.accent, CircleShape)
                                .clickable { appManager.pushScreen(Screen.NewChat) }
                                .testTag("chatListNewChatButton"),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            imageVector = IrisIcons.NewChat,
                            contentDescription = "New chat",
                            tint = MaterialTheme.colorScheme.onPrimary,
                        )
                    }
                },
            )
        },
    ) { padding ->
        val nearbySnapshot = nearbyTick.let { nearbyService?.snapshot }
        LazyColumn(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background),
        ) {
            if (nearbyService != null && nearbySnapshot != null) {
                item(key = "nearby") {
                    Column(modifier = Modifier.fillMaxWidth()) {
                        IrisChatListRow(
                            title = "Nearby",
                            preview =
                                if (nearbySnapshot.visible && nearbySnapshot.peerCount > 0) {
                                    if (nearbySnapshot.peerCount == 1) "1 nearby" else "${nearbySnapshot.peerCount} nearby"
                                } else {
                                    nearbySnapshot.status
                                },
                            timeLabel = null,
                            leadingContent = {
                                NearbyChatIcon(visible = nearbySnapshot.visible)
                            },
                            unreadCount = 0,
                            lastMessageMine = false,
                            lastDelivery = null,
                            onClick = {
                                onNearbyClick()
                                nearbyTick += 1
                            },
                            modifier = Modifier.testTag("nearbyChatRow"),
                        )
                        if (nearbySnapshot.visible && nearbySnapshot.peers.isNotEmpty()) {
                            NearbyPeerStrip(
                                appManager = appManager,
                                appState = appState,
                                peers = nearbySnapshot.peers,
                                modifier = Modifier.padding(start = 70.dp, end = 16.dp, bottom = 10.dp),
                            )
                        }
                        IrisDivider(modifier = Modifier.padding(start = 70.dp))
                    }
                }
            }
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
                    val avatarData by rememberNhashImageData(appManager, chat.pictureUrl)
                    val avatarUrl =
                        chat.pictureUrl
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
                            imageUrl = avatarUrl,
                            imageData = avatarData,
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

@Composable
private fun NearbyPeerStrip(
    appManager: AppManager,
    appState: AppState,
    peers: List<IrisNearbyService.Peer>,
    modifier: Modifier = Modifier,
) {
    LazyRow(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        items(peers, key = { it.id }) { peer ->
            NearbyPeerChip(
                appManager = appManager,
                appState = appState,
                peer = peer,
            )
        }
    }
}

@Composable
private fun NearbyPeerChip(
    appManager: AppManager,
    appState: AppState,
    peer: IrisNearbyService.Peer,
) {
    val avatarData by rememberNhashImageData(appManager, peer.pictureUrl)
    val avatarUrl =
        peer.pictureUrl
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
    Column(
        modifier =
            Modifier
                .width(64.dp)
                .clickable(enabled = peer.ownerPubkeyHex != null) {
                    peer.ownerPubkeyHex?.let(appManager::createChat)
                }
                .testTag("nearbyPeer-${peer.id.take(12)}"),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        IrisAvatar(
            label = peer.name,
            size = 42.dp,
            imageUrl = avatarUrl,
            imageData = avatarData,
        )
        Text(
            text = peer.name,
            modifier = Modifier.padding(top = 4.dp),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

@Composable
private fun NearbyChatIcon(visible: Boolean) {
    val palette = IrisTheme.palette
    Box(
        modifier =
            Modifier
                .size(42.dp)
                .background(if (visible) palette.accent else palette.panelAlt, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = IrisIcons.Nearby,
            contentDescription = null,
            tint = if (visible) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(24.dp),
        )
    }
}

@Composable
internal fun rememberNhashImageData(
    appManager: AppManager,
    pictureUrl: String?,
) = produceState<ByteArray?>(initialValue = null, pictureUrl) {
    val nhash = parseNhashUri(pictureUrl)
    value = if (nhash == null) null else appManager.resolveHashtreePictureBytes(nhash)
}

internal fun parseNhashUri(value: String?): String? {
    val trimmed = value?.trim().orEmpty()
    val prefix = when {
        trimmed.startsWith("htree://") -> "htree://"
        trimmed.startsWith("nhash://") -> "nhash://"
        else -> return null
    }
    return trimmed
        .removePrefix(prefix)
        .substringBefore("/")
        .takeIf(String::isNotBlank)
}
