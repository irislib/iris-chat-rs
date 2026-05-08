package to.iris.chat.ui.screens

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.roundToInt
import java.util.concurrent.ConcurrentHashMap
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot
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
    var pendingDeleteChat by remember { mutableStateOf<ChatThreadSnapshot?>(null) }
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
                            preview = nearbyPreview(nearbySnapshot),
                            timeLabel = null,
                            leadingContent = {
                                NearbyChatIcon(visible = nearbySnapshot.visible || nearbySnapshot.localNetworkVisible)
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
                        SwipeableChatListRow(
                            chat = chat,
                            onToggleUnread = {
                                appManager.dispatch(
                                    AppAction.SetChatUnread(chat.chatId, chat.unreadCount == 0uL),
                                )
                            },
                            onTogglePin = {
                                appManager.dispatch(
                                    AppAction.SetChatPinned(chat.chatId, !chat.isPinned),
                                )
                            },
                            onToggleMute = {
                                appManager.dispatch(
                                    AppAction.SetChatMuted(chat.chatId, !chat.isMuted),
                                )
                            },
                            onDeleteRequest = { pendingDeleteChat = chat },
                        ) {
                            IrisChatListRow(
                                title = chat.displayName,
                                isMuted = chat.isMuted,
                                isPinned = chat.isPinned,
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
                        }
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

        pendingDeleteChat?.let { chat ->
            AlertDialog(
                onDismissRequest = { pendingDeleteChat = null },
                title = { Text("Delete chat?") },
                confirmButton = {
                    Button(
                        onClick = {
                            appManager.dispatch(AppAction.DeleteChat(chat.chatId))
                            pendingDeleteChat = null
                        },
                    ) {
                        Text("Delete")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { pendingDeleteChat = null }) {
                        Text("Cancel")
                    }
                },
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeableChatListRow(
    chat: ChatThreadSnapshot,
    onToggleUnread: () -> Unit,
    onTogglePin: () -> Unit,
    onToggleMute: () -> Unit,
    onDeleteRequest: () -> Unit,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    val rowOffsetDistancePx = with(density) { ChatSwipeActionsWidth.toPx() }
    val flingThresholdPx = with(density) { 700.dp.toPx() }
    var targetOffsetPx by remember(chat.chatId) { mutableFloatStateOf(0f) }
    var isDragging by remember(chat.chatId) { mutableStateOf(false) }
    val rowOffsetPx by animateFloatAsState(
        targetValue = targetOffsetPx,
        animationSpec = if (isDragging) snap() else tween(durationMillis = 160),
        label = "chatSwipeOffset",
    )
    val dragState =
        rememberDraggableState { delta ->
            targetOffsetPx = (targetOffsetPx + delta).coerceIn(-rowOffsetDistancePx, rowOffsetDistancePx)
        }

    LaunchedEffect(rowOffsetDistancePx) {
        targetOffsetPx = targetOffsetPx.coerceIn(-rowOffsetDistancePx, rowOffsetDistancePx)
    }

    Box(modifier = Modifier.fillMaxWidth()) {
        if (rowOffsetPx > 1f || targetOffsetPx > 1f) {
            Row(
                modifier =
                    Modifier
                        .matchParentSize()
                        .background(IrisTheme.palette.panelAlt)
                        .padding(horizontal = 12.dp),
                horizontalArrangement = Arrangement.Start,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ChatSwipeActionButton(
                    label = if (chat.unreadCount > 0uL) "Read" else "Unread",
                    icon = if (chat.unreadCount > 0uL) IrisIcons.MarkRead else IrisIcons.MarkUnread,
                    color = IrisTheme.palette.accent,
                    onClick = {
                        onToggleUnread()
                        targetOffsetPx = 0f
                    },
                )
                Spacer(modifier = Modifier.width(8.dp))
                ChatSwipeActionButton(
                    label = if (chat.isPinned) "Unpin" else "Pin",
                    icon = IrisIcons.Pin,
                    color = IrisTheme.palette.accentAlt,
                    onClick = {
                        onTogglePin()
                        targetOffsetPx = 0f
                    },
                )
            }
        } else if (rowOffsetPx < -1f || targetOffsetPx < -1f) {
            Row(
                modifier =
                    Modifier
                        .matchParentSize()
                        .background(IrisTheme.palette.panelAlt)
                        .padding(horizontal = 12.dp),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ChatSwipeActionButton(
                    label = if (chat.isMuted) "Unmute" else "Mute",
                    icon = if (chat.isMuted) IrisIcons.Notifications else IrisIcons.NotificationsOff,
                    color = IrisTheme.palette.accent,
                    onClick = {
                        onToggleMute()
                        targetOffsetPx = 0f
                    },
                )
                Spacer(modifier = Modifier.width(8.dp))
                ChatSwipeActionButton(
                    label = "Delete",
                    icon = IrisIcons.DeleteForever,
                    color = MaterialTheme.colorScheme.error,
                    onClick = {
                        onDeleteRequest()
                        targetOffsetPx = 0f
                    },
                )
            }
        }

        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .offset { IntOffset(rowOffsetPx.roundToInt(), 0) }
                    .background(MaterialTheme.colorScheme.background),
        ) {
            Box(
                modifier =
                    Modifier.draggable(
                        state = dragState,
                        orientation = Orientation.Horizontal,
                        startDragImmediately = targetOffsetPx != 0f,
                        onDragStarted = { isDragging = true },
                        onDragStopped = { velocity ->
                            isDragging = false
                            targetOffsetPx =
                                when {
                                    velocity > flingThresholdPx -> rowOffsetDistancePx
                                    velocity < -flingThresholdPx -> -rowOffsetDistancePx
                                    targetOffsetPx > rowOffsetDistancePx * 0.45f -> rowOffsetDistancePx
                                    targetOffsetPx < -rowOffsetDistancePx * 0.45f -> -rowOffsetDistancePx
                                    else -> 0f
                                }
                        },
                    ),
            ) {
                content()
            }
        }
    }
}

private val ChatSwipeActionsWidth = 176.dp

@Composable
private fun ChatSwipeActionButton(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    color: Color,
    onClick: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .width(72.dp)
                .height(58.dp)
                .clickable(onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(imageVector = icon, contentDescription = label, tint = color)
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            color = color,
        )
    }
}

private fun nearbyPreview(snapshot: IrisNearbyService.Snapshot): String =
    when {
        snapshot.peers.isNotEmpty() -> nearbyPeerSummary(snapshot.peers)
        !snapshot.visible &&
            !snapshot.localNetworkVisible &&
            (!snapshot.bluetoothPermissionGranted || !snapshot.localNetworkPermissionGranted) -> "Click to enable"
        !snapshot.visible && !snapshot.localNetworkVisible -> "Off"
        snapshot.localNetworkVisible && snapshot.localNetworkStatus in nearbyLanBlockingStatuses ->
            nearbyWifiStatusLabel(snapshot.localNetworkStatus)
        !snapshot.localNetworkVisible && snapshot.status in nearbyBlockingStatuses -> snapshot.status
        else -> "No users nearby"
    }

private fun nearbyPeerSummary(peers: List<IrisNearbyService.Peer>): String {
    val names = peers.map { it.name.trim().ifEmpty { "Someone" } }
    return when (names.size) {
        1 -> "${names[0]} nearby"
        2 -> "${names[0]} and ${names[1]} nearby"
        3 -> "${names[0]}, ${names[1]} and ${names[2]} nearby"
        else -> {
            val otherCount = names.size - 3
            val suffix = if (otherCount == 1) "other" else "others"
            "${names.take(3).joinToString(", ")} and $otherCount $suffix nearby"
        }
    }
}

private val nearbyBlockingStatuses =
    setOf(
        "No Bluetooth access",
        "Bluetooth off",
        "Bluetooth unavailable",
        "Advertise unavailable",
        "Advertise failed",
        "Scan failed",
        "Connect failed",
    )

private val nearbyLanBlockingStatuses =
    setOf(
        "No local network access",
        "Local network unavailable",
        "Local network failed",
    )

private fun nearbyWifiStatusLabel(status: String): String =
    when (status) {
        "No local network access" -> "No Wi-Fi access"
        "Local network unavailable" -> "Wi-Fi unavailable"
        "Local network failed" -> "Wi-Fi failed"
        else -> status
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
) = produceState(
    initialValue = parseNhashUri(pictureUrl)?.let { NhashImageDataCache.get(it) },
    pictureUrl,
) {
    val nhash = parseNhashUri(pictureUrl)
    if (nhash == null) {
        value = null
        return@produceState
    }
    NhashImageDataCache.get(nhash)?.let {
        value = it
        return@produceState
    }
    value =
        appManager.resolveHashtreePictureBytes(nhash)
            ?.also { NhashImageDataCache.put(nhash, it) }
}

private object NhashImageDataCache {
    private val images = ConcurrentHashMap<String, ByteArray>()

    fun get(nhash: String): ByteArray? = images[nhash]

    fun put(nhash: String, data: ByteArray) {
        images[nhash] = data
    }
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
