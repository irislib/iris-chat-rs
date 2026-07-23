package to.iris.chat.ui.screens

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.ripple
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.rounded.Add
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import java.util.concurrent.ConcurrentHashMap
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatInputShortcut
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.MessageSearchHit
import to.iris.chat.rust.Screen
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.classifyChatInput
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisSearchViewMoreRow
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.formatRelativeTime
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ChatListScreen(
    appManager: AppManager,
    appState: AppState,
    nearbyService: IrisNearbyService? = null,
    onNearbyClick: () -> Unit = {},
    onNearbyLongClick: () -> Unit = {},
    onNearbyPeerLongClick: (String) -> Unit = {},
) {
    var pendingDeleteChat by remember { mutableStateOf<ChatThreadSnapshot?>(null) }
    val account = appState.account

    var searchQuery by remember { mutableStateOf("") }
    val haptics = rememberIrisHapticFeedback()
    val profileButtonInteractionSource = remember { MutableInteractionSource() }
    val trimmedQuery = searchQuery.trim()
    val searchActive = trimmedQuery.isNotEmpty()
    var expandedSearchSections by remember(trimmedQuery) { mutableStateOf(emptySet<SearchSection>()) }
    var messageSearchLimit by remember(trimmedQuery) { mutableStateOf(InitialMessageSearchLimit) }
    var searchResults by remember { mutableStateOf<SearchResultSnapshot?>(null) }

    // Keep Rust/SQLite work out of composition. Search is refreshed only for
    // user-driven inputs: the query itself and explicit "view more" requests.
    LaunchedEffect(trimmedQuery, messageSearchLimit, appState.userDiscoveryRevision) {
        searchResults =
            if (trimmedQuery.isEmpty()) {
                null
            } else {
                appManager.search(trimmedQuery, limit = messageSearchLimit)
            }
    }

    // Mirrors NewChatScreen's auto-proceed behaviour: when the user
    // pastes a full npub or invite URL into the search bar, dispatch
    // the matching action without forcing them to tap the shortcut
    // row. Partial input never classifies as a shortcut, so this is
    // safe on every keystroke.
    LaunchedEffect(trimmedQuery) {
        if (trimmedQuery.isEmpty()) return@LaunchedEffect
        when (val shortcut = classifyChatInput(trimmedQuery)) {
            is ChatInputShortcut.DirectPeer -> {
                searchQuery = ""
                appManager.dispatch(AppAction.CreateChat(shortcut.peerInput))
            }
            is ChatInputShortcut.Invite -> {
                searchQuery = ""
                appManager.dispatch(AppAction.AcceptInvite(shortcut.inviteInput))
            }
            null -> Unit
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
                                    .size(48.dp)
                                    .clip(CircleShape)
                                    .testTag("chatListProfileButton")
                                    .clickable(
                                        interactionSource = profileButtonInteractionSource,
                                        indication = ripple(bounded = false, radius = 24.dp),
                                    ) {
                                        haptics.press()
                                        appManager.pushScreen(Screen.Settings)
                                    },
                            contentAlignment = Alignment.Center,
                        ) {
                            IrisAvatar(
                                label = account.displayName,
                                emphasize = true,
                                size = 32.dp,
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
                    IconButton(
                        onClick = {
                            haptics.press()
                            appManager.pushScreen(Screen.NewChat)
                        },
                        modifier = Modifier.testTag("chatListNewChatButton"),
                    ) {
                        Icon(
                            imageVector = Icons.Rounded.Add,
                            contentDescription = "New chat",
                            tint = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.size(26.dp),
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
                val results = searchResults?.takeIf { it.matchesSearchRequest(trimmedQuery) }
                val findingPeople = appState.userDiscoverySyncing && (results == null || results.people.isEmpty())
                val emptyResults = results == null
                    || (results.people.isEmpty()
                        && results.contacts.isEmpty()
                        && results.groups.isEmpty()
                        && results.messages.isEmpty()
                        && results.shortcut == null)
                if (emptyResults && !findingPeople) {
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
                    results?.shortcut?.let { shortcut ->
                        item(key = "search-shortcut") {
                            ChatInputShortcutRow(
                                appManager = appManager,
                                shortcut = shortcut,
                                onNavigate = { searchQuery = "" },
                            )
                        }
                    }
                    if (findingPeople) {
                        item(key = "section-people-finding") {
                            SearchSectionHeader("People")
                            Text(
                                text = "Finding people…",
                                modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
                                style = MaterialTheme.typography.bodyMedium,
                                color = IrisTheme.palette.muted,
                            )
                        }
                    }
                    if (results?.people?.isNotEmpty() == true) {
                        item(key = "section-people") { SearchSectionHeader("People") }
                        val people = visibleSearchRows(
                            rows = results.people,
                            section = SearchSection.People,
                            expandedSections = expandedSearchSections,
                            initialCount = 7,
                        )
                        items(people, key = { "p:${it.ownerPubkeyHex}" }) { person ->
                            FollowedPersonSearchRow(appManager, appState, person)
                        }
                        if (results.people.size > people.size) {
                            item(key = "section-people-more") {
                                IrisSearchViewMoreRow {
                                    expandedSearchSections = expandedSearchSections + SearchSection.People
                                }
                            }
                        }
                    }
                    if (results?.contacts?.isNotEmpty() == true) {
                        item(key = "section-contacts") { SearchSectionHeader("Contacts") }
                        val contacts = visibleSearchRows(
                            rows = results.contacts,
                            section = SearchSection.Contacts,
                            expandedSections = expandedSearchSections,
                            initialCount = 7,
                        )
                        items(contacts, key = { "c:${it.chatId}" }) { chat ->
                            SearchChatRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                            )
                        }
                        if (results.contacts.size > contacts.size) {
                            item(key = "section-contacts-more") {
                                IrisSearchViewMoreRow {
                                    expandedSearchSections = expandedSearchSections + SearchSection.Contacts
                                }
                            }
                        }
                    }
                    if (results?.groups?.isNotEmpty() == true) {
                        item(key = "section-groups") { SearchSectionHeader("Groups") }
                        val groups = visibleSearchRows(
                            rows = results.groups,
                            section = SearchSection.Groups,
                            expandedSections = expandedSearchSections,
                            initialCount = 7,
                        )
                        items(groups, key = { "g:${it.chatId}" }) { chat ->
                            SearchChatRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                            )
                        }
                        if (results.groups.size > groups.size) {
                            item(key = "section-groups-more") {
                                IrisSearchViewMoreRow {
                                    expandedSearchSections = expandedSearchSections + SearchSection.Groups
                                }
                            }
                        }
                    }
                    if (results?.messages?.isNotEmpty() == true) {
                        item(key = "section-messages") { SearchSectionHeader("Messages") }
                        val messages = visibleSearchRows(
                            rows = results.messages,
                            section = SearchSection.Messages,
                            expandedSections = expandedSearchSections,
                            initialCount = 20,
                        )
                        items(messages, key = { "m:${it.chatId}:${it.messageId}" }) { hit ->
                            MessageSearchHitRow(
                                appManager = appManager,
                                appState = appState,
                                hit = hit,
                            )
                        }
                        val canShowFetchedMessages = results.messages.size > messages.size
                        val canFetchMoreMessages = SearchSection.Messages in expandedSearchSections &&
                            results.messages.size.toUInt() >= messageSearchLimit
                        if (canShowFetchedMessages || canFetchMoreMessages) {
                            item(key = "section-messages-more") {
                                IrisSearchViewMoreRow {
                                    if (SearchSection.Messages in expandedSearchSections) {
                                        messageSearchLimit = nextMessageSearchLimit(messageSearchLimit)
                                    } else {
                                        expandedSearchSections = expandedSearchSections + SearchSection.Messages
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                val nearby = nearbyService.takeIf { appState.preferences.nearbyShowInChatList }
                val showNearby = nearby != null
                val pinnedChats = appState.chatList.filter { it.isPinned }
                val unpinnedChats = appState.chatList.filter { !it.isPinned }
                val visibleSectionCount =
                    (if (showNearby) 1 else 0) +
                        (if (pinnedChats.isNotEmpty()) 1 else 0) +
                        (if (unpinnedChats.isNotEmpty() || appState.chatList.isEmpty()) 1 else 0)

                if (nearby != null) {
                    if (visibleSectionCount > 1) {
                        item(key = "section-nearby") { SearchSectionHeader("Nearby") }
                    }
                    item(key = "nearby") {
                        NearbyChatListItem(
                            appManager = appManager,
                            nearbyEnabled = appState.preferences.nearbyEnabled,
                            nearbyBluetoothEnabled = appState.preferences.nearbyBluetoothEnabled,
                            knownDirectChatNames = appState.knownDirectChatNames(),
                            service = nearby,
                            onClick = onNearbyClick,
                            onLongClick = onNearbyLongClick,
                            onPeerLongClick = onNearbyPeerLongClick,
                        )
                    }
                }
                if (appState.chatList.isEmpty()) {
                    if (visibleSectionCount > 1) {
                        item(key = "section-chats") { SearchSectionHeader("Chats") }
                    }
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
                    if (pinnedChats.isNotEmpty()) {
                        if (visibleSectionCount > 1) {
                            item(key = "section-pinned") { SearchSectionHeader("Pinned") }
                        }
                        items(pinnedChats, key = { it.chatId }) { chat ->
                            ChatListConversationRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                                onDeleteRequest = { pendingDeleteChat = it },
                            )
                        }
                    }
                    if (unpinnedChats.isNotEmpty()) {
                        if (visibleSectionCount > 1) {
                            item(key = "section-chats") { SearchSectionHeader("Chats") }
                        }
                        items(unpinnedChats, key = { it.chatId }) { chat ->
                            ChatListConversationRow(
                                appManager = appManager,
                                appState = appState,
                                chat = chat,
                                onDeleteRequest = { pendingDeleteChat = it },
                            )
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
                            haptics.confirm()
                            appManager.dispatch(AppAction.DeleteChat(chat.chatId))
                            pendingDeleteChat = null
                        },
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.error,
                            contentColor = MaterialTheme.colorScheme.onError,
                        ),
                    ) {
                        Text("Delete")
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            haptics.press()
                            pendingDeleteChat = null
                        },
                        colors = ButtonDefaults.textButtonColors(
                            contentColor = MaterialTheme.colorScheme.onSurface,
                        ),
                    ) {
                        Text("Cancel")
                    }
                },
            )
        }
    }
}

private enum class SearchSection {
    People,
    Contacts,
    Groups,
    Messages,
}

private val ChatPreviewWhitespace = Regex("\\s+")

private fun <T> visibleSearchRows(
    rows: List<T>,
    section: SearchSection,
    expandedSections: Set<SearchSection>,
    initialCount: Int,
): List<T> = if (section in expandedSections) rows else rows.take(initialCount)

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
    var lastHapticDirection by remember(chat.chatId) { mutableStateOf(0) }
    val haptics = rememberIrisHapticFeedback()
    val rowOffsetPx by animateFloatAsState(
        targetValue = targetOffsetPx,
        animationSpec = if (isDragging) snap() else tween(durationMillis = 160),
        label = "chatSwipeOffset",
    )
    val dragState =
        rememberDraggableState { delta ->
            targetOffsetPx = (targetOffsetPx + delta).coerceIn(-rowOffsetDistancePx, rowOffsetDistancePx)
            val direction =
                when {
                    targetOffsetPx > rowOffsetDistancePx * 0.45f -> 1
                    targetOffsetPx < -rowOffsetDistancePx * 0.45f -> -1
                    else -> 0
                }
            if (direction != 0 && direction != lastHapticDirection) {
                haptics.longPress()
                lastHapticDirection = direction
            } else if (direction == 0) {
                lastHapticDirection = 0
            }
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
                    color = MaterialTheme.colorScheme.onBackground,
                    onClick = {
                        onToggleUnread()
                        targetOffsetPx = 0f
                        lastHapticDirection = 0
                    },
                )
                Spacer(modifier = Modifier.width(8.dp))
                ChatSwipeActionButton(
                    label = if (chat.isPinned) "Unpin" else "Pin",
                    icon = IrisIcons.Pin,
                    color = MaterialTheme.colorScheme.onBackground,
                    onClick = {
                        onTogglePin()
                        targetOffsetPx = 0f
                        lastHapticDirection = 0
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
                    color = MaterialTheme.colorScheme.onBackground,
                    onClick = {
                        onToggleMute()
                        targetOffsetPx = 0f
                        lastHapticDirection = 0
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
                        lastHapticDirection = 0
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
                                    else -> {
                                        lastHapticDirection = 0
                                        0f
                                    }
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
private val NearbyActiveBlue = Color(0xFF2267F5)
private val NearbyChatRowHeight = 88.dp

@Composable
private fun ChatSwipeActionButton(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    color: Color,
    onClick: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Column(
        modifier =
            Modifier
                .width(72.dp)
                .height(58.dp)
                .clip(RoundedCornerShape(14.dp))
                .clickable(
                    interactionSource = interactionSource,
                    indication = ripple(),
                    onClick = {
                        haptics.press()
                        onClick()
                    },
                ),
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
@OptIn(ExperimentalFoundationApi::class)
private fun NearbyChatListItem(
    appManager: AppManager,
    nearbyEnabled: Boolean,
    nearbyBluetoothEnabled: Boolean,
    knownDirectChatNames: Map<String, String>,
    service: IrisNearbyService,
    onClick: () -> Unit,
    onLongClick: () -> Unit,
    onPeerLongClick: (String) -> Unit,
) {
    val snapshot by rememberNearbySnapshotState(service)
    val nearbyActive = nearbyEnabled && (nearbyBluetoothEnabled || snapshot.localNetworkVisible)
    val visiblePeers =
        if (nearbyEnabled) {
            rememberSortedNearbyPeers(
                peers = snapshot.peers,
                knownDirectChatIds = knownDirectChatNames.keys,
                bluetoothPeerIds = snapshot.bluetoothPeers.mapTo(mutableSetOf()) { it.id },
                localNetworkPeerIds = snapshot.localNetworkPeers.mapTo(mutableSetOf()) { it.id },
            )
        } else {
            emptyList()
        }
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(NearbyChatRowHeight)
                .combinedClickable(
                    interactionSource = interactionSource,
                    indication = null,
                    hapticFeedbackEnabled = false,
                    onClick = {
                        haptics.press()
                        onClick()
                    },
                    onLongClick = {
                        haptics.longPress()
                        onLongClick()
                    },
                )
                .padding(horizontal = 16.dp, vertical = 10.dp)
                .testTag("nearbyChatRow"),
        horizontalArrangement = Arrangement.spacedBy(20.dp),
        verticalAlignment = Alignment.Top,
    ) {
        NearbyChatIcon(enabled = nearbyActive)

        if (visiblePeers.isEmpty()) {
            Box(
                modifier =
                    Modifier
                        .weight(1f)
                        .height(48.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                Text(
                    text =
                        when {
                            nearbyActive -> "No users nearby"
                            else -> "Tap to enable"
                        },
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        } else {
            Row(
                modifier =
                    Modifier
                        .weight(1f)
                        .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.Top,
            ) {
                visiblePeers.forEach { peer ->
                    val displayName = nearbyPeerResolvedName(peer, knownDirectChatNames)
                    NearbyPeerAvatar(
                        peer = peer,
                        displayName = displayName,
                        onClick = {
                            peer.ownerPubkeyHex
                                ?.takeIf { it.isNotBlank() }
                                ?.let { owner ->
                                    if (owner.lowercase() in knownDirectChatNames) {
                                        appManager.openChat(owner)
                                    } else {
                                        onPeerLongClick(owner)
                                    }
                                }
                        },
                        onLongClick = { ownerPubkeyHex ->
                            haptics.longPress()
                            onPeerLongClick(ownerPubkeyHex)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun ChatListConversationRow(
    appManager: AppManager,
    appState: AppState,
    chat: ChatThreadSnapshot,
    onDeleteRequest: (ChatThreadSnapshot) -> Unit,
) {
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
            onDeleteRequest = { onDeleteRequest(chat) },
        ) {
            IrisChatListRow(
                title = chat.displayName,
                isMuted = chat.isMuted,
                isPinned = chat.isPinned,
                preview = chat.chatListPreview(),
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

@Composable
@OptIn(ExperimentalFoundationApi::class)
private fun NearbyPeerAvatar(
    peer: IrisNearbyService.Peer,
    displayName: String,
    onClick: () -> Unit,
    onLongClick: (String) -> Unit,
) {
    val name = nearbyPeerDisplayName(displayName)
    val interactionSource = remember(peer.id) { MutableInteractionSource() }
    val ownerPubkeyHex = peer.ownerPubkeyHex?.takeIf { it.isNotBlank() }
    Column(
        modifier =
            Modifier
                .width(64.dp)
                .clip(RoundedCornerShape(8.dp))
                .combinedClickable(
                    enabled = ownerPubkeyHex != null,
                    interactionSource = interactionSource,
                    indication = null,
                    hapticFeedbackEnabled = false,
                    onClick = onClick,
                    onLongClick = ownerPubkeyHex?.let { owner ->
                        { onLongClick(owner) }
                    },
                ),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        IrisAvatar(
            label = displayName,
            size = 48.dp,
            imageUrl = peer.pictureUrl,
        )
        Text(
            text = name,
            style = MaterialTheme.typography.labelSmall,
            color = IrisTheme.palette.muted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun NearbyChatIcon(enabled: Boolean) {
    val palette = IrisTheme.palette
    Box(
        modifier =
            Modifier
                .size(48.dp)
                .background(if (enabled) NearbyActiveBlue else palette.panelAlt, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = IrisIcons.Nearby,
            contentDescription = null,
            tint = if (enabled) MaterialTheme.colorScheme.onPrimary else palette.muted,
            modifier = Modifier.size(24.dp),
        )
    }
}

private fun nearbyPeerDisplayName(name: String): String {
    val trimmed = name.trim().ifEmpty { "Nearby" }
    return if (trimmed.length <= 14) trimmed else trimmed.take(13) + "…"
}

private fun AppState.knownDirectChatNames(): Map<String, String> =
    chatList
        .asSequence()
        .filter { it.kind == ChatKind.DIRECT }
        .associate { it.chatId.lowercase() to it.displayName.trim().ifEmpty { "Nearby" } }

private fun nearbyPeerResolvedName(
    peer: IrisNearbyService.Peer,
    knownDirectChatNames: Map<String, String>,
): String {
    val owner = peer.ownerPubkeyHex?.trim()?.lowercase()
    if (owner != null) {
        knownDirectChatNames[owner]?.let { return it }
    }
    return peer.name.trim().ifEmpty { "Nearby" }
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
    val focusManager = LocalFocusManager.current
    val keyboardController = LocalSoftwareKeyboardController.current
    val haptics = rememberIrisHapticFeedback()
    var focused by remember { mutableStateOf(false) }
    val closeSearch: () -> Unit = {
        haptics.press()
        if (query.isNotEmpty()) {
            onClear()
        }
        focusManager.clearFocus()
        keyboardController?.hide()
    }

    TextField(
        value = query,
        onValueChange = onQueryChange,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .heightIn(min = 48.dp)
                .onFocusChanged { focused = it.isFocused }
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
            if (focused || query.isNotEmpty()) {
                IconButton(
                    onClick = closeSearch,
                    modifier = Modifier.testTag("chatListCloseSearchButton"),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Clear,
                        contentDescription = "Close search",
                        tint = IrisTheme.palette.muted,
                    )
                }
            }
        },
        singleLine = true,
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        shape = RoundedCornerShape(24.dp),
        colors = irisTextFieldColors(containerColor = IrisTheme.palette.panelRaised),
    )
}

internal fun ChatThreadSnapshot.chatListPreview(): String {
    val draftPreview = draft.trim().replace(ChatPreviewWhitespace, " ")
    return when {
        isTyping -> "Typing"
        draftPreview.isNotEmpty() -> "Draft: $draftPreview"
        else -> lastMessagePreview ?: subtitle.orEmpty()
    }
}

@Composable
private fun ChatInputShortcutRow(
    appManager: AppManager,
    shortcut: ChatInputShortcut,
    onNavigate: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    val (title, subtitle, icon, action) = when (shortcut) {
        is ChatInputShortcut.DirectPeer -> Quad(
            "Start chat",
            shortcut.display,
            Icons.Filled.Search,
            { appManager.dispatch(AppAction.CreateChat(shortcut.peerInput)) },
        )
        is ChatInputShortcut.Invite -> Quad(
            "Accept invite",
            shortcut.display,
            Icons.Filled.Search,
            { appManager.dispatch(AppAction.AcceptInvite(shortcut.inviteInput)) },
        )
    }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(
                    interactionSource = interactionSource,
                    indication = null,
                    onClick = {
                        haptics.press()
                        onNavigate()
                        action()
                    },
                )
                .padding(horizontal = 16.dp, vertical = 12.dp)
                .testTag("chatListSearchShortcut"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(40.dp)
                    .background(IrisTheme.palette.accent, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimary,
            )
        }
        Spacer(modifier = Modifier.width(12.dp))
        Column(modifier = Modifier.fillMaxWidth()) {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.SemiBold),
                color = MaterialTheme.colorScheme.onBackground,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.labelMedium,
                color = IrisTheme.palette.muted,
                maxLines = 1,
            )
        }
    }
}

private data class Quad<A, B, C, D>(val a: A, val b: B, val c: C, val d: D)

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
        onClick = { appManager.openChatAtMessage(hit.chatId, hit.messageId) },
        modifier = Modifier.testTag("messageHit-${hit.messageId.take(12)}"),
    )
}
