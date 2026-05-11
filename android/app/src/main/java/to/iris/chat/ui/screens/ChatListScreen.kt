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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.input.ImeAction
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
import to.iris.chat.rust.MessageSearchHit
import to.iris.chat.rust.Screen
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisChatListRow
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
    var pendingDeleteChat by remember { mutableStateOf<ChatThreadSnapshot?>(null) }
    val account = appState.account

    var searchQuery by remember { mutableStateOf("") }
    val trimmedQuery = searchQuery.trim()
    val searchActive = trimmedQuery.isNotEmpty()
    // Re-evaluate whenever the query, chat list, or revision change so
    // an incoming message shows up in matched contacts immediately. The
    // search() call is sub-millisecond against FTS5 and only inspects
    // in-memory state for the contacts/groups buckets.
    val searchResults: SearchResultSnapshot? by remember(searchQuery, appState.rev) {
        derivedStateOf {
            if (searchActive) {
                appManager.search(trimmedQuery)
            } else {
                null
            }
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
        LazyColumn(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background),
        ) {
            item(key = "chatListSearch") {
                ChatListSearchField(
                    query = searchQuery,
                    onQueryChange = { searchQuery = it },
                    onClear = { searchQuery = "" },
                )
            }
            if (searchActive) {
                val results = searchResults
                if (results == null || results.contacts.isEmpty() && results.groups.isEmpty() && results.messages.isEmpty()) {
                    item(key = "chatListSearchEmpty") {
                        Box(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 28.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                text = "No matches",
                                style = MaterialTheme.typography.bodyMedium,
                                color = IrisTheme.palette.muted,
                            )
                        }
                    }
                } else {
                    if (results.contacts.isNotEmpty()) {
                        item(key = "section-contacts") { SearchSectionHeader("Contacts") }
                        items(results.contacts, key = { "c:${it.chatId}" }) { chat ->
                            SearchChatRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                            )
                        }
                    }
                    if (results.groups.isNotEmpty()) {
                        item(key = "section-groups") { SearchSectionHeader("Groups") }
                        items(results.groups, key = { "g:${it.chatId}" }) { chat ->
                            SearchChatRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                            )
                        }
                    }
                    if (results.messages.isNotEmpty()) {
                        item(key = "section-messages") { SearchSectionHeader("Messages") }
                        items(results.messages, key = { "m:${it.chatId}:${it.messageId}" }) { hit ->
                            MessageSearchHitRow(
                                appManager = appManager,
                                appState = appState,
                                hit = hit,
                            )
                        }
                    }
                }
            } else {
                if (nearbyService != null) {
                    item(key = "nearby") {
                        NearbyChatListItem(
                            service = nearbyService,
                            onClick = onNearbyClick,
                        )
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
                                timeLabel = formatRelativeTime(chat.lastMessageAtSecs?.toLong(), System.currentTimeMillis()),
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
                    }
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

@Composable
private fun NearbyChatListItem(
    service: IrisNearbyService,
    onClick: () -> Unit,
) {
    var tick by remember { mutableStateOf(0) }

    LaunchedEffect(service) {
        while (true) {
            delay(1_000L)
            tick += 1
        }
    }

    val snapshot = tick.let { service.snapshot }
    Column(modifier = Modifier.fillMaxWidth()) {
        IrisChatListRow(
            title = "Nearby",
            preview = nearbyPreview(snapshot),
            timeLabel = null,
            leadingContent = {
                NearbyChatIcon(visible = snapshot.visible || snapshot.localNetworkVisible)
            },
            previewLeading =
                if (snapshot.peers.isNotEmpty()) {
                    { NearbyAvatarStack(peers = snapshot.peers.take(3)) }
                } else {
                    null
                },
            unreadCount = 0,
            lastMessageMine = false,
            lastDelivery = null,
            onClick = onClick,
            modifier = Modifier.testTag("nearbyChatRow"),
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

// Small avatar group shown inline with the "Boromir nearby" subtitle so the
// faces of the people who are actually around appear next to their names,
// rather than replacing the wireless icon in the leading slot. Each avatar
// gets a thin ring in the surface color to keep overlapping faces
// visually separated, same trick as iOS.
@Composable
private fun NearbyAvatarStack(peers: List<IrisNearbyService.Peer>) {
    // Stays smaller than the bodyMedium line height so the row's preview
    // row doesn't grow taller when the stack appears.
    val avatarSize = 16.dp
    val overlap = 6.dp
    val stride = avatarSize - overlap
    val stackWidth = stride * (peers.size - 1) + avatarSize
    val ringColor = MaterialTheme.colorScheme.background
    Box(modifier = Modifier.size(width = stackWidth, height = avatarSize)) {
        peers.forEachIndexed { index, peer ->
            Box(
                modifier =
                    Modifier
                        .align(Alignment.CenterStart)
                        .offset(x = stride * index)
                        .size(avatarSize)
                        .background(ringColor, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                IrisAvatar(
                    label = peer.name.ifBlank { "?" },
                    size = avatarSize - 2.dp,
                    imageUrl = peer.pictureUrl,
                )
            }
        }
    }
}

@Composable
private fun NearbyChatIcon(visible: Boolean) {
    val palette = IrisTheme.palette
    Box(
        modifier =
            Modifier
                .size(48.dp)
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChatListSearchField(
    query: String,
    onQueryChange: (String) -> Unit,
    onClear: () -> Unit,
) {
    TextField(
        value = query,
        onValueChange = onQueryChange,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp)
                .testTag("chatListSearchField"),
        placeholder = { Text("Search chats, groups, messages") },
        leadingIcon = {
            Icon(
                imageVector = Icons.Filled.Search,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
            )
        },
        trailingIcon = {
            if (query.isNotEmpty()) {
                IconButton(onClick = onClear) {
                    Icon(
                        imageVector = Icons.Filled.Clear,
                        contentDescription = "Clear search",
                        tint = IrisTheme.palette.muted,
                    )
                }
            }
        },
        singleLine = true,
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        shape = RoundedCornerShape(14.dp),
        colors =
            TextFieldDefaults.colors(
                focusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                unfocusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                focusedIndicatorColor = Color.Transparent,
                unfocusedIndicatorColor = Color.Transparent,
                disabledIndicatorColor = Color.Transparent,
            ),
    )
}

@Composable
private fun SearchSectionHeader(title: String) {
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
private fun SearchChatRow(
    appManager: AppManager,
    appState: AppState,
    chat: ChatThreadSnapshot,
) {
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
    IrisChatListRow(
        title = chat.displayName,
        isMuted = chat.isMuted,
        isPinned = chat.isPinned,
        preview = chat.lastMessagePreview ?: chat.subtitle.orEmpty(),
        timeLabel = formatRelativeTime(chat.lastMessageAtSecs?.toLong(), System.currentTimeMillis()),
        imageUrl = avatarUrl,
        imageData = avatarData,
        unreadCount = chat.unreadCount.toLong(),
        lastMessageMine = chat.lastMessageIsOutgoing == true,
        lastDelivery = chat.lastMessageDelivery,
        onClick = { appManager.openChat(chat.chatId) },
    )
}

@Composable
private fun MessageSearchHitRow(
    appManager: AppManager,
    appState: AppState,
    hit: MessageSearchHit,
) {
    val avatarData by rememberNhashImageData(appManager, hit.chatPictureUrl)
    val avatarUrl =
        hit.chatPictureUrl
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
    IrisChatListRow(
        title = hit.chatDisplayName,
        isMuted = false,
        isPinned = false,
        preview = hit.body,
        timeLabel = formatRelativeTime(hit.createdAtSecs.toLong(), System.currentTimeMillis()),
        imageUrl = avatarUrl,
        imageData = avatarData,
        unreadCount = 0L,
        lastMessageMine = false,
        lastDelivery = null,
        onClick = { appManager.openChat(hit.chatId) },
        modifier = Modifier.testTag("messageHit-${hit.messageId.take(12)}"),
    )
}
