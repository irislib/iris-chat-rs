package to.iris.chat.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.stopScroll
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Check
import androidx.compose.ui.draw.clip
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.ripple
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInParent
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.core.AppManager
import to.iris.chat.nearby.IrisNearbyService
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.OutgoingAttachment
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.peerInputToNpub
import to.iris.chat.rust.proxiedImageUrl
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.IrisInlineAction
import to.iris.chat.ui.components.IrisSearchViewMoreRow
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.IrisChatListRow
import to.iris.chat.ui.components.IrisDivider
import to.iris.chat.ui.components.IrisTopBar
import to.iris.chat.ui.components.formatRelativeTime
import androidx.compose.foundation.lazy.items
import to.iris.chat.ui.components.formatTimelineDay
import to.iris.chat.ui.components.irisTextFieldColors
import to.iris.chat.ui.components.isSameTimelineDay
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

private val DisappearingMessageOptions =
    listOf(
        "Off" to null,
        "5 minutes" to 300uL,
        "1 hour" to 3_600uL,
        "24 hours" to 86_400uL,
        "1 week" to 604_800uL,
        "1 month" to 2_592_000uL,
        "3 months" to 7_776_000uL,
    )

// Compact label for the chat header subtitle when disappearing-messages is on.
// Tries the canonical menu options first so the wording matches what the user
// picked, then falls back to a generic unit-based string for any odd value.
private fun disappearingLabel(seconds: ULong): String {
    DisappearingMessageOptions.firstOrNull { it.second == seconds }?.let { return it.first }
    return when {
        seconds < 3_600uL -> "${seconds / 60uL} min"
        seconds < 86_400uL -> "${seconds / 3_600uL} h"
        seconds < 604_800uL -> "${seconds / 86_400uL} d"
        seconds < 2_592_000uL -> "${seconds / 604_800uL} wk"
        else -> "${seconds / 2_592_000uL} mo"
    }
}

@Composable
fun ChatScreen(
    appManager: AppManager,
    chatId: String,
    nearbyService: IrisNearbyService? = null,
    openInfoOnStart: Boolean = false,
    onInfoOpenConsumed: () -> Unit = {},
) {
    // Subscribe to per-slice flows so this screen only recomposes when
    // *its* state slice changed. Tapping into the consolidated
    // `appManager.state` would force a full recompose every time a
    // relay event landed in another chat, which on Android debug took
    // 1-2 s of UI thread time per relay event.
    val currentChat by appManager.currentChat.collectAsStateWithLifecycle()
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val busy by appManager.busy.collectAsStateWithLifecycle()
    val router by appManager.router.collectAsStateWithLifecycle()
    val chatListSnapshots by appManager.chatList.collectAsStateWithLifecycle()
    val isAppForegrounded by appManager.appForegrounded.collectAsStateWithLifecycle()
    val lastUserActivityAtSecs by appManager.lastUserActivityAtSecs.collectAsStateWithLifecycle()

    val context = LocalContext.current
    val focusManager = LocalFocusManager.current
    val keyboardController = LocalSoftwareKeyboardController.current
    val chat = currentChat?.takeIf { it.chatId == chatId }
    var draft by remember(chatId) { mutableStateOf("") }
    var lastPersistedDraft by remember(chatId) { mutableStateOf<String?>(null) }
    var selectedAttachments by remember(chatId) { mutableStateOf<List<PickedAttachment>>(emptyList()) }
    val listState = rememberLazyListState()
    val coroutineScope = rememberCoroutineScope()
    val haptics = rememberIrisHapticFeedback()
    val jumpInteractionSource = remember { MutableInteractionSource() }
    var shouldFollowLatest by remember(chatId) { mutableStateOf(true) }
    var forceScrollToLatest by remember(chatId) { mutableStateOf(false) }
    var initialScrollPending by remember(chatId) { mutableStateOf(true) }
    var observedMessageCount by remember(chatId) { mutableStateOf(0) }
    var replyTarget by remember(chatId) { mutableStateOf<ChatMessageSnapshot?>(null) }
    var imageViewerItem by remember(chatId) { mutableStateOf<ImageViewerItem?>(null) }
    var lastTypingSentMs by remember(chatId) { mutableStateOf(0L) }
    var hasSentTyping by remember(chatId) { mutableStateOf(false) }
    val latestDraft by rememberUpdatedState(draft)
    val latestLastPersistedDraft by rememberUpdatedState(lastPersistedDraft)
    val latestHasSentTyping by rememberUpdatedState(hasSentTyping)
    var directChatInfoOpen by remember(chatId) { mutableStateOf(false) }
    var inChatSearchOpen by remember(chatId) { mutableStateOf(false) }
    var composerBounds by remember { mutableStateOf<Rect?>(null) }
    val composerFocusRequester = remember { FocusRequester() }
    val backUnreadCount by remember(chatId) {
        derivedStateOf {
            chatListSnapshots
                .filter { it.chatId != chatId }
                .fold(0uL) { total, thread -> total + thread.unreadCount }
        }
    }
    val attachmentPicker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
            if (uris.isEmpty()) {
                return@rememberLauncherForActivityResult
            }
            coroutineScope.launch {
                val attachments =
                    withContext(Dispatchers.IO) {
                        uris.mapNotNull { uri -> copyAttachmentToCache(context, uri) }
                    }
                if (attachments.isNotEmpty()) {
                    selectedAttachments = selectedAttachments + attachments
                }
            }
        }
    val showJumpToBottom by remember(chat?.messages?.size, listState) {
        derivedStateOf {
            val total = chat?.messages?.size ?: 0
            if (total == 0) {
                false
            } else {
                val lastVisible = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
                lastVisible < total - 1
            }
        }
    }

    LaunchedEffect(chatId) {
        shouldFollowLatest = true
        forceScrollToLatest = false
        initialScrollPending = true
        observedMessageCount = 0
    }

    LaunchedEffect(chatId, openInfoOnStart) {
        if (openInfoOnStart) {
            directChatInfoOpen = true
            onInfoOpenConsumed()
        }
    }

    // Keep the composer aligned with the persisted thread draft without
    // clobbering unsaved local typing. This handles the initial load, a
    // draft restored after the screen appears, and external clears after send.
    LaunchedEffect(chatId, chat?.draft) {
        val persisted = chat?.draft ?: return@LaunchedEffect
        val previousPersisted = lastPersistedDraft
        when {
            previousPersisted == null -> {
                draft = persisted
                lastPersistedDraft = persisted
            }
            persisted != previousPersisted && draft == previousPersisted -> {
                draft = persisted
                lastPersistedDraft = persisted
            }
            else -> {
                lastPersistedDraft = persisted
            }
        }
    }

    // Debounced persist: 500ms after the user stops typing, push the
    // current text into the thread's `draft` column. The Rust side
    // dedups against the previous value so no-op writes are cheap.
    LaunchedEffect(chatId, draft) {
        val currentDraft = draft
        if (lastPersistedDraft == currentDraft) {
            return@LaunchedEffect
        }
        delay(500)
        if (lastPersistedDraft != currentDraft) {
            appManager.dispatch(AppAction.SetChatDraft(chatId, currentDraft))
            lastPersistedDraft = currentDraft
        }
    }

    LaunchedEffect(listState, chat?.messages?.size) {
        snapshotFlow {
            val total = chat?.messages?.size ?: 0
            if (total == 0) {
                true
            } else {
                val lastVisible = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
                lastVisible >= total - 2
            }
        }
            .distinctUntilChanged()
            .collect { isNearBottom ->
                shouldFollowLatest = isNearBottom
            }
    }

    val pendingScrollMessage by appManager.pendingScrollMessage.collectAsStateWithLifecycle()

    LaunchedEffect(chatId, chat?.messages?.size, chat?.messages?.lastOrNull()?.id, forceScrollToLatest, pendingScrollMessage) {
        val messages = chat?.messages.orEmpty()
        val total = messages.size
        if (total == 0) {
            initialScrollPending = true
            observedMessageCount = 0
            forceScrollToLatest = false
            return@LaunchedEffect
        }
        // Search-hit jump: when the user tapped a "Messages" row, the
        // manager set a one-shot target id. If the chat's now showing
        // that message, scroll to it instead of bottom and clear the
        // flag so back-and-forth navigation doesn't keep snapping.
        val target = pendingScrollMessage
        if (target != null) {
            val targetIndex = messages.indexOfFirst { it.id == target }
            if (targetIndex >= 0) {
                observedMessageCount = total
                initialScrollPending = false
                shouldFollowLatest = false
                forceScrollToLatest = false
                listState.scrollToItem(targetIndex)
                appManager.consumePendingScrollMessage()
                return@LaunchedEffect
            }
            appManager.loadChatAroundMessage(chatId, target)
        }
        val previousTotal = observedMessageCount
        val lastVisible = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
        val wasNearPreviousBottom = previousTotal == 0 || lastVisible >= previousTotal - 2
        val messageCountIncreased = total > previousTotal
        val shouldScroll =
            initialScrollPending ||
                forceScrollToLatest ||
                (messageCountIncreased && (shouldFollowLatest || wasNearPreviousBottom))
        observedMessageCount = total
        if (shouldScroll) {
            if (initialScrollPending) {
                listState.scrollToItem(total - 1)
            } else {
                listState.animateScrollToItem(total - 1)
            }
            initialScrollPending = false
            shouldFollowLatest = true
        }
        if (forceScrollToLatest) {
            forceScrollToLatest = false
        }
    }

    // Repin to the bottom when the LAST bubble's height grows (a reaction
    // landed, an attachment image lazy-loaded taller than its placeholder,
    // a quote preview rendered) while we were already following the latest.
    // We watch `last.size` rather than `lastBottom > viewportEnd` because
    // the latter also turns true the moment the user scrolls up, which
    // would force-snap them back down — exactly the bug iOS just fixed.
    // `last.size` only changes when the last bubble actually resizes.
    LaunchedEffect(listState, chatId) {
        var previousLastSize = 0
        snapshotFlow {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull() ?: return@snapshotFlow null
            if (last.index != info.totalItemsCount - 1) return@snapshotFlow null
            Triple(info.totalItemsCount, last.size, last.offset + last.size > info.viewportEndOffset)
        }
            .distinctUntilChanged()
            .collect { snap ->
                if (snap == null) {
                    previousLastSize = 0
                    return@collect
                }
                val (total, lastSize, overflowsViewport) = snap
                val grew = previousLastSize > 0 && lastSize > previousLastSize
                previousLastSize = lastSize
                if (initialScrollPending || !shouldFollowLatest || total == 0) return@collect
                if (grew && overflowsViewport) {
                    listState.scrollToItem(total - 1)
                }
            }
    }

    val unseenIncomingIds =
        remember(chat?.messages) {
            chat
                ?.messages
                ?.filter { message -> !message.isOutgoing && message.delivery != DeliveryState.SEEN }
                ?.map { message -> message.id }
                .orEmpty()
        }

    LaunchedEffect(chatId, unseenIncomingIds, isAppForegrounded, lastUserActivityAtSecs) {
        if (
            unseenIncomingIds.isNotEmpty() &&
                appManager.canMarkActiveChatSeen(isAppForegrounded, lastUserActivityAtSecs)
        ) {
            appManager.dispatch(AppAction.MarkMessagesSeen(chatId, unseenIncomingIds))
        }
    }

    DisposableEffect(chatId) {
        onDispose {
            if (latestHasSentTyping) {
                hasSentTyping = false
                lastTypingSentMs = 0L
                appManager.dispatch(AppAction.StopTyping(chatId))
            }
            // Flush any pending draft on the way out so the latest
            // text always hits SQLite before this screen tears down.
            val draftOnDispose = latestDraft
            if (latestLastPersistedDraft != draftOnDispose) {
                appManager.dispatch(AppAction.SetChatDraft(chatId, draftOnDispose))
            }
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        topBar = {
            val chatHeaderAvatarBytes by rememberNhashImageData(appManager, chat?.pictureUrl)
            IrisTopBar(
                title =
                    when {
                        chat?.kind == ChatKind.GROUP && chat.subtitle != null ->
                            "${chat.displayName} · ${chat.subtitle}"
                        else -> chat?.displayName ?: "Chat"
                    },
                subtitle =
                    when {
                        chat?.messageTtlSeconds?.let { it > 0uL } == true ->
                            disappearingLabel(chat.messageTtlSeconds!!)
                        chat?.isMuted == true -> "muted"
                        else -> null
                    },
                subtitleIcon =
                    when {
                        chat?.messageTtlSeconds?.let { it > 0uL } == true -> Icons.Rounded.Schedule
                        chat?.isMuted == true -> IrisIcons.NotificationsOff
                        else -> null
                    },
                onBack = { appManager.navigateBack() },
                backBadgeCount = backUnreadCount,
                titleAccessoryLeading =
                    if (chat != null) {
                        {
                            IrisAvatar(
                                label = chat.displayName,
                                size = 36.dp,
                                emphasize = false,
                                imageUrl =
                                    chat.pictureUrl
                                        ?.takeIf {
                                            it.startsWith("http://") || it.startsWith("https://")
                                        }
                                        ?.let { url ->
                                            proxiedImageUrl(
                                                originalSrc = url,
                                                preferences = preferences,
                                                width = 72u,
                                                height = 72u,
                                                square = true,
                                            )
                                        },
                                imageData = chatHeaderAvatarBytes,
                            )
                        }
                    } else {
                        null
                    },
                onTitleClick =
                    chat?.let { current ->
                        current.groupId?.let { groupId ->
                            { appManager.pushScreen(Screen.GroupDetails(groupId)) }
                        } ?: { directChatInfoOpen = true }
                    },
                actions = {
                    if (chat != null) {
                        IconButton(
                            onClick = { inChatSearchOpen = true },
                            modifier = Modifier.testTag("chatHeaderSearchButton"),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.Search,
                                contentDescription = "Search in this chat",
                                tint = MaterialTheme.colorScheme.onBackground,
                            )
                        }
                    }
                },
            )
        },
    ) { padding ->
        if (chat == null) {
            Box(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding),
                contentAlignment = Alignment.Center,
            ) {
                Text("Loading chat…")
            }
            return@Scaffold
        }
        if (directChatInfoOpen) {
            DirectChatInfoScreen(
                appManager = appManager,
                chatId = chatId,
                nearbyService = nearbyService,
                onBack = { directChatInfoOpen = false },
            )
            return@Scaffold
        }
        val visibleMessages = chat.messages
        var scrollDateHeaderVisible by remember(chatId) { mutableStateOf(false) }
        var scrollDateHeaderLabel by remember(chatId) { mutableStateOf<String?>(null) }

        LaunchedEffect(chatId, visibleMessages, listState) {
            snapshotFlow { listState.isScrollInProgress to listState.firstVisibleItemIndex }
                .distinctUntilChanged()
                .collectLatest { (isScrolling, firstVisibleIndex) ->
                    visibleMessages.getOrNull(firstVisibleIndex)?.let { message ->
                        scrollDateHeaderLabel = formatTimelineDay(message.createdAtSecs.toLong())
                    }
                    if (isScrolling) {
                        scrollDateHeaderVisible = scrollDateHeaderLabel != null
                    } else {
                        delay(900)
                        if (!listState.isScrollInProgress) {
                            scrollDateHeaderVisible = false
                        }
                    }
                }
        }

        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background)
                    .clearFocusOnTapOutside(composerBounds) {
                        focusManager.clearFocus(force = true)
                        keyboardController?.hide()
                    },
        ) {
            Column(modifier = Modifier.fillMaxSize()) {
                Box(
                    modifier =
                        Modifier
                            .weight(1f)
                            .fillMaxWidth(),
                ) {
                    LazyColumn(
                        state = listState,
                        modifier =
                            Modifier
                                .fillMaxSize()
                                .testTag("chatTimeline"),
                        verticalArrangement = Arrangement.spacedBy(0.dp, Alignment.Bottom),
                    ) {
                        itemsIndexed(visibleMessages, key = { _, message -> message.id }) { index, message ->
                            if (index == 0) {
                                LaunchedEffect(chat.chatId, message.id) {
                                    appManager.loadOlderMessages(chat.chatId)
                                }
                            }
                            val previous = visibleMessages.getOrNull(index - 1)
                            val next = visibleMessages.getOrNull(index + 1)
                            val showDayChip =
                                previous == null ||
                                    !isSameTimelineDay(
                                        previous.createdAtSecs.toLong(),
                                        message.createdAtSecs.toLong(),
                                    )
                            val isFirstInCluster = startsMessageCluster(previous, message, chat.kind)
                            val isLastInCluster = next == null || startsMessageCluster(message, next, chat.kind)

                            if (showDayChip) {
                                TimelineDaySeparator(
                                    text = formatTimelineDay(message.createdAtSecs.toLong()),
                                    floating = false,
                                )
                            }

                            MessageBubble(
                                message = message,
                                chatKind = chat.kind,
                                isFirstInCluster = isFirstInCluster,
                                isLastInCluster = isLastInCluster,
                                reactions = message.reactions,
                                onReply = {
                                    replyTarget = message
                                    composerFocusRequester.requestFocus()
                                },
                                onForward = {
                                    appManager.startForward(forwardableMessageText(message))
                                },
                                onReact = { emoji ->
                                    appManager.dispatch(
                                        AppAction.ToggleReaction(
                                            chatId = chatId,
                                            messageId = message.id,
                                            emoji = emoji,
                                        ),
                                    )
                                },
                                onDelete = {
                                    appManager.dispatch(
                                        AppAction.DeleteLocalMessage(
                                            chatId = chatId,
                                            messageId = message.id,
                                        ),
                                    )
                                    if (replyTarget?.id == message.id) {
                                        replyTarget = null
                                    }
                                },
                                onScrollToQuote = {
                                    val parsedReply = parseReplyEncodedMessage(message.body).reply
                                    if (parsedReply != null) {
                                        val target = chat.messages.indexOfLast { candidate ->
                                            candidate.author == parsedReply.author &&
                                                replySnippet(candidate) == parsedReply.body &&
                                                candidate.createdAtSecs <= message.createdAtSecs &&
                                                candidate.id != message.id
                                        }
                                        if (target >= 0) {
                                            coroutineScope.launch {
                                                listState.animateScrollToItem(target)
                                            }
                                        }
                                    }
                                },
                                downloadAttachment = { attachment ->
                                    appManager.downloadAttachment(attachment)
                                },
                                onOpenImage = { data, attachment ->
                                    val imageAttachments = message.attachments.filter { it.isImage }
                                    val initialIndex = imageAttachments
                                        .indexOfFirst { it.htreeUrl == attachment.htreeUrl }
                                        .coerceAtLeast(0)
                                    imageViewerItem = ImageViewerItem(
                                        attachments = imageAttachments,
                                        initialIndex = initialIndex,
                                        initialData = data,
                                        senderName = if (message.isOutgoing) "You" else message.author,
                                        createdAtSecs = message.createdAtSecs.toLong(),
                                    )
                                },
                                chat = chat,
                                appManager = appManager,
                            )
                        }
                    }

                    if (scrollDateHeaderVisible && scrollDateHeaderLabel != null) {
                        scrollDateHeaderLabel?.let { label ->
                            TimelineDaySeparator(
                                text = label,
                                floating = true,
                                modifier =
                                    Modifier
                                        .align(Alignment.TopCenter)
                                        .padding(top = 12.dp)
                                        .testTag("chatFloatingDaySeparator"),
                            )
                        }
                    }

                    if (chat.typingIndicators.isNotEmpty()) {
                        TypingIndicatorBubble(
                            names = chat.typingIndicators.map { indicator -> indicator.displayName },
                            modifier =
                                Modifier
                                    .align(Alignment.BottomStart)
                                    .padding(start = 14.dp, end = 14.dp, bottom = 10.dp),
                        )
                    }
                }

                replyTarget?.let { reply ->
                    ReplyComposerStrip(
                        message = reply,
                        onCancel = { replyTarget = null },
                    )
                }

                ComposerBar(
                    draft = draft,
                    selectedAttachments = selectedAttachments,
                    isSending = busy.sendingMessage,
                    isUploading = busy.uploadingAttachment,
                    uploadFraction = busy.uploadProgress?.let { progress ->
                        if (progress.totalBytes > 0u) {
                            (progress.bytesUploaded.toDouble() / progress.totalBytes.toDouble())
                                .toFloat()
                                .coerceIn(0f, 1f)
                        } else {
                            null
                        }
                    },
                    focusRequester = composerFocusRequester,
                    modifier = Modifier.onGloballyPositioned { coordinates ->
                        composerBounds = coordinates.boundsInParent()
                    },
                    onDraftChange = { value ->
                        draft = value
                        if (value.isBlank()) {
                            if (hasSentTyping) {
                                hasSentTyping = false
                                lastTypingSentMs = 0L
                                appManager.dispatch(AppAction.StopTyping(chatId))
                            }
                        } else {
                            val nowMs = System.currentTimeMillis()
                            if (nowMs - lastTypingSentMs >= 3_000L) {
                                lastTypingSentMs = nowMs
                                hasSentTyping = true
                                appManager.dispatch(AppAction.SendTyping(chatId))
                            }
                        }
                    },
                    onAttach = { attachmentPicker.launch(arrayOf("*/*")) },
                    onRemoveAttachment = { attachment ->
                        selectedAttachments = selectedAttachments - attachment
                    },
                    onSend = {
                        shouldFollowLatest = true
                        forceScrollToLatest = true
                        val outgoingDraft = replyEncodedMessage(replyTarget, draft.trim())
                        replyTarget = null
                        if (selectedAttachments.isEmpty()) {
                            appManager.sendText(chatId, outgoingDraft)
                        } else {
                            appManager.sendAttachments(
                                chatId = chatId,
                                attachments =
                                    selectedAttachments.map { attachment ->
                                        OutgoingAttachment(
                                            filePath = attachment.path,
                                            filename = attachment.filename,
                                        )
                                    },
                                caption = outgoingDraft,
                            )
                            selectedAttachments = emptyList()
                        }
                        draft = ""
                        lastPersistedDraft = ""
                        appManager.dispatch(AppAction.SetChatDraft(chatId, ""))
                        if (hasSentTyping) {
                            hasSentTyping = false
                            lastTypingSentMs = 0L
                            appManager.dispatch(AppAction.StopTyping(chatId))
                        }
                    },
                )
            }

            if (showJumpToBottom) {
                // Signal-style FAB: 36dp circular surface raised over the
                // timeline with a soft shadow, sitting just above the
                // composer (~12dp gap, plus the composer's own 70dp).
                Surface(
                    modifier =
                        Modifier
                            .align(Alignment.BottomEnd)
                            .padding(end = 12.dp, bottom = 92.dp)
                            .size(36.dp)
                            .clip(CircleShape)
                            .clickable(
                                interactionSource = jumpInteractionSource,
                                indication = ripple(radius = 18.dp),
                            ) {
                                haptics.press()
                                coroutineScope.launch {
                                    listState.stopScroll()
                                    val total = chat.messages.size
                                    if (total > 0) {
                                        listState.animateScrollToItem(total - 1)
                                    }
                                }
                            }
                            .testTag("chatJumpToBottom"),
                    color = IrisTheme.palette.panelRaised,
                    shape = CircleShape,
                    shadowElevation = 4.dp,
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            imageVector = Icons.Rounded.KeyboardArrowDown,
                            contentDescription = "Scroll to bottom",
                            tint = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }

            imageViewerItem?.let { item ->
                ImageViewerDialog(
                    item = item,
                    downloadAttachment = { attachment -> appManager.downloadAttachment(attachment) },
                    onForward = { attachment ->
                        appManager.startForward(forwardableAttachmentText(attachment))
                    },
                    onDismiss = { imageViewerItem = null },
                )
            }

            if (inChatSearchOpen) {
                InChatSearchSheet(
                    appManager = appManager,
                    chatId = chatId,
                    chatDisplayName = chat.displayName,
                    onDismiss = { inChatSearchOpen = false },
                )
            }
        }
    }
}

@Composable
private fun TimelineDaySeparator(
    text: String,
    floating: Boolean,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(top = if (floating) 0.dp else 12.dp, bottom = if (floating) 0.dp else 13.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (floating) {
            Surface(
                color = IrisTheme.palette.panelRaised,
                shape = RoundedCornerShape(18.dp),
                shadowElevation = 2.dp,
                tonalElevation = 0.dp,
            ) {
                Text(
                    text = text,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        } else {
            Text(
                text = text,
                modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun DirectChatInfoScreen(
    appManager: AppManager,
    chatId: String,
    nearbyService: IrisNearbyService?,
    onBack: () -> Unit,
) {
    val currentChat by appManager.currentChat.collectAsStateWithLifecycle()
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val chat = currentChat?.takeIf { it.chatId == chatId } ?: return
    val nearbySnapshot = nearbyService?.let { rememberNearbySnapshotState(it).value }
    val nearbyStatus = nearbyProfileStatus(preferences.nearbyEnabled, nearbySnapshot, chat.chatId)
    val avatarBytes by rememberNhashImageData(appManager, chat.pictureUrl)
    var advancedOpen by remember(chatId) { mutableStateOf(false) }
    var profileDebug by remember(chatId) { mutableStateOf<PeerProfileDebugSnapshot?>(null) }
    var commonGroups by remember(chatId) { mutableStateOf<List<ChatThreadSnapshot>>(emptyList()) }
    var nicknameDraft by remember(chatId) { mutableStateOf(chat.nickname.orEmpty()) }
    var editingNickname by remember(chatId) { mutableStateOf(false) }
    val proxiedAvatarUrl =
        chat.pictureUrl
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
            ?.let { url ->
                proxiedImageUrl(
                    originalSrc = url,
                    preferences = preferences,
                    width = 192u,
                    height = 192u,
                    square = true,
                )
            }

    LaunchedEffect(advancedOpen, chatId) {
        if (advancedOpen && profileDebug == null) {
            profileDebug = appManager.peerProfileDebug(chatId)
        }
    }
    LaunchedEffect(chatId) {
        commonGroups = appManager.mutualGroups(chatId)
    }
    LaunchedEffect(chatId, chat.nickname) {
        nicknameDraft = chat.nickname.orEmpty()
    }

    BackHandler {
        onBack()
    }

    Surface(
        modifier =
            Modifier
                .fillMaxSize()
                .testTag("directChatInfoScreen"),
        color = MaterialTheme.colorScheme.background,
    ) {
        Scaffold(
            containerColor = MaterialTheme.colorScheme.background,
            topBar = {
                IrisTopBar(
                    title = chat.displayName,
                    onBack = onBack,
                )
            },
        ) { padding ->
            // Chat info exposes the peer's hex pubkey + npub +
            // their relay debug counters — make them all
            // long-press-to-copy. SelectionContainer is inert for
            // buttons, IconButtons, and the avatar; only Text
            // children pick up selection.
            SelectionContainer(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IrisAvatar(
                            label = chat.displayName,
                            size = 72.dp,
                            emphasize = true,
                            imageUrl = proxiedAvatarUrl,
                            imageData = avatarBytes,
                        )
                        Column(
                            modifier = Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                text = chat.displayName,
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = FontWeight.Bold,
                            )
                            chat.subtitle?.takeIf { it.isNotBlank() }?.let { subtitle ->
                                Text(
                                    text = subtitle,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = IrisTheme.palette.muted,
                                )
                            }
                        }
                    }
                    val clipboard = rememberIrisClipboard()
                    ProfileAboutCard(
                        about = chat.about,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    NearbyProfileStatusCard(
                        status = nearbyStatus,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    ContactNicknameCard(
                        chat = chat,
                        nicknameDraft = nicknameDraft,
                        onNicknameChange = { nicknameDraft = it },
                        onSave = {
                            appManager.dispatch(
                                AppAction.SetContactNickname(chatId, nicknameDraft),
                            )
                            editingNickname = false
                        },
                        onRemove = {
                            nicknameDraft = ""
                            editingNickname = false
                            appManager.dispatch(AppAction.SetContactNickname(chatId, ""))
                        },
                        editing = editingNickname,
                        onToggleEditing = { editingNickname = !editingNickname },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    if (commonGroups.isNotEmpty()) {
                        CommonGroupsCard(
                            appManager = appManager,
                            groups = commonGroups,
                            onBack = onBack,
                        )
                    }
                    IrisInlineAction(
                        text = "Copy user ID",
                        onClick = { clipboard.setText("User ID", peerInputToNpub(chatId)) },
                        modifier = Modifier.testTag("directChatCopyUserIdButton"),
                    ) {
                        Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                    }
                    IrisInlineAction(
                        text = if (chat.isMuted) "Unmute chat" else "Mute chat",
                        onClick = {
                            appManager.dispatch(AppAction.SetChatMuted(chatId, !chat.isMuted))
                        },
                        modifier = Modifier.testTag("directChatMuteButton"),
                    ) {
                        Icon(
                            imageVector =
                                if (chat.isMuted) {
                                    IrisIcons.Notifications
                                } else {
                                    IrisIcons.NotificationsOff
                                },
                            contentDescription = null,
                        )
                    }
                    DisappearingMessagesCard(
                        currentTtlSeconds = chat.messageTtlSeconds,
                        onSelect = { ttlSeconds ->
                            appManager.dispatch(AppAction.SetChatMessageTtl(chatId, ttlSeconds))
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    DirectChatAdvancedCard(
                        debug = profileDebug,
                        expanded = advancedOpen,
                        onToggle = { advancedOpen = !advancedOpen },
                        modifier = Modifier.testTag("directChatAdvancedCard"),
                    )
                    IrisInlineAction(
                        text = "Delete chat",
                        onClick = {
                            appManager.dispatch(AppAction.DeleteChat(chatId))
                            onBack()
                        },
                        modifier = Modifier.testTag("directChatDeleteButton"),
                    ) {
                        Icon(
                            imageVector = IrisIcons.DeleteForever,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ProfileAboutCard(
    about: String?,
    modifier: Modifier = Modifier,
) {
    val text = about?.trim()?.takeIf { it.isNotEmpty() } ?: return
    val linkColor = MaterialTheme.colorScheme.primary
    IrisSectionCard(modifier = modifier.testTag("directChatAboutCard")) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = IrisIcons.Edit,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurface,
                modifier =
                    Modifier
                        .padding(top = 2.dp)
                        .size(22.dp),
            )
            Text(
                text = remember(text, linkColor) { linkHighlightedText(text, linkColor) },
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private val UrlTextRegex = Regex("""\b(?:https?://|www\.)\S+""")

private fun linkHighlightedText(
    text: String,
    linkColor: Color,
): AnnotatedString =
    buildAnnotatedString {
        var cursor = 0
        for (match in UrlTextRegex.findAll(text)) {
            val range = match.range
            if (range.first > cursor) {
                append(text.substring(cursor, range.first))
            }
            withStyle(
                SpanStyle(
                    color = linkColor,
                    textDecoration = TextDecoration.Underline,
                ),
            ) {
                append(match.value)
            }
            cursor = range.last + 1
        }
        if (cursor < text.length) {
            append(text.substring(cursor))
        }
    }

@Composable
private fun NearbyProfileStatusCard(
    status: String?,
    modifier: Modifier = Modifier,
) {
    val text = status ?: return
    IrisSectionCard(modifier = modifier.testTag("directChatNearbyStatusCard")) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = IrisIcons.Nearby,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(22.dp),
            )
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = "Nearby now",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = IrisTheme.palette.muted,
                )
            }
        }
    }
}

private fun nearbyProfileStatus(
    nearbyEnabled: Boolean,
    snapshot: IrisNearbyService.Snapshot?,
    ownerPubkeyHex: String,
): String? {
    if (!nearbyEnabled || snapshot == null) {
        return null
    }
    val onBluetooth =
        snapshot.bluetoothPeers.any { peer ->
            peer.ownerPubkeyHex?.equals(ownerPubkeyHex, ignoreCase = true) == true
        }
    val onWifi =
        snapshot.localNetworkPeers.any { peer ->
            peer.ownerPubkeyHex?.equals(ownerPubkeyHex, ignoreCase = true) == true
        }
    return when {
        onBluetooth && onWifi -> "Bluetooth and Wi-Fi"
        onBluetooth -> "Bluetooth"
        onWifi -> "Wi-Fi"
        else -> null
    }
}

@Composable
private fun ContactNicknameCard(
    chat: to.iris.chat.rust.CurrentChatSnapshot,
    nicknameDraft: String,
    onNicknameChange: (String) -> Unit,
    onSave: () -> Unit,
    onRemove: () -> Unit,
    editing: Boolean,
    onToggleEditing: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val storedNickname = chat.nickname?.trim().orEmpty()
    val primaryName = storedNickname.ifEmpty { chat.displayName.trim() }
    val profileName =
        chat.profileName
            ?.trim()
            ?.takeIf { it.isNotEmpty() && !it.equals(primaryName, ignoreCase = true) }
    val summary = storedNickname.ifEmpty { "Add nickname" }

    IrisSectionCard(modifier = modifier) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onToggleEditing)
                    .testTag("directChatNicknameRow"),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Nickname",
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = summary,
                style = MaterialTheme.typography.bodyLarge,
                color = if (storedNickname.isEmpty()) IrisTheme.palette.muted else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        profileName?.let { name ->
            IrisDivider()
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("directChatProfileNameRow"),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Profile name",
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = name,
                    style = MaterialTheme.typography.bodyLarge,
                    color = IrisTheme.palette.muted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (editing) {
            IrisDivider()
            TextField(
                value = nicknameDraft,
                onValueChange = onNicknameChange,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("directChatNicknameField"),
                label = { Text("Nickname") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                colors = irisTextFieldColors(),
            )
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IrisInlineAction(
                    text = "Save",
                    onClick = onSave,
                    modifier = Modifier.testTag("directChatSaveNicknameButton"),
                ) {
                    Icon(imageVector = IrisIcons.Check, contentDescription = null)
                }
                if (storedNickname.isNotEmpty()) {
                    IrisInlineAction(
                        text = "Remove",
                        onClick = onRemove,
                        modifier = Modifier.testTag("directChatRemoveNicknameButton"),
                    ) {
                        Icon(imageVector = IrisIcons.Close, contentDescription = null)
                    }
                }
            }
        }
    }
}

@Composable
private fun CommonGroupsCard(
    appManager: AppManager,
    groups: List<ChatThreadSnapshot>,
    onBack: () -> Unit,
) {
    IrisSectionCard {
        Text(
            text = "Groups in common",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
        )
        groups.forEachIndexed { index, group ->
            CommonGroupRow(
                appManager = appManager,
                group = group,
                onBack = onBack,
            )
            if (index < groups.lastIndex) {
                IrisDivider(modifier = Modifier.padding(start = 50.dp))
            }
        }
    }
}

@Composable
private fun CommonGroupRow(
    appManager: AppManager,
    group: ChatThreadSnapshot,
    onBack: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable {
                    val groupId = groupIdFromChatId(group.chatId) ?: return@clickable
                    haptics.press()
                    onBack()
                    appManager.dispatch(AppAction.PushScreen(Screen.GroupDetails(groupId)))
                }
                .padding(vertical = 2.dp)
                .testTag("directChatCommonGroup-${group.chatId.take(12)}"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(
            label = group.displayName,
            size = 38.dp,
            imageUrl = group.pictureUrl,
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Text(
                text = group.displayName,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${group.memberCount} people",
                style = MaterialTheme.typography.bodyMedium,
                color = IrisTheme.palette.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(
            imageVector = IrisIcons.ChevronRight,
            contentDescription = null,
            tint = IrisTheme.palette.muted,
            modifier = Modifier.size(22.dp),
        )
    }
}

private fun groupIdFromChatId(chatId: String): String? {
    val trimmed = chatId.trim()
    val prefix = "group:"
    if (!trimmed.startsWith(prefix, ignoreCase = true)) {
        return null
    }
    return trimmed.drop(prefix.length).trim().takeIf { it.isNotEmpty() }
}

@Composable
private fun DirectChatAdvancedCard(
    debug: PeerProfileDebugSnapshot?,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    IrisSectionCard(modifier = modifier) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        interactionSource = interactionSource,
                        indication = null,
                    ) {
                        haptics.press()
                        onToggle()
                    },
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = IrisIcons.Devices,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = "Debug",
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Icon(
                imageVector = IrisIcons.ChevronRight,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.graphicsLayer { rotationZ = if (expanded) 90f else 0f },
            )
        }

        if (expanded) {
            if (debug == null) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    strokeWidth = 2.dp,
                    color = IrisTheme.palette.accent,
                )
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    ProfileDebugRow("Sessions", debug.sessionCount.toString())
                    ProfileDebugRow("Active sessions", debug.activeSessionCount.toString())
                    ProfileDebugRow("Receiving sessions", debug.receivingSessionCount.toString())
                    ProfileDebugRow("Known devices", debug.knownDeviceCount.toString())
                    ProfileDebugRow("Device roster", debug.rosterDeviceCount.toString())
                    ProfileDebugRow("Tracked senders", debug.trackedSenderCount.toString())
                    ProfileDebugRow("Recent handshakes", debug.recentHandshakeDeviceCount.toString())
                    ProfileDebugRow("Last handshake", lastHandshakeText(debug.lastHandshakeAtSecs))
                    ProfileDebugRow("Message tracking", if (debug.trackedForMessages) "On" else "Off")
                    ProfileDebugRow("User ID", debug.ownerNpub, monospaced = true)
                    ProfileDebugRow("Public key", debug.ownerPubkeyHex, monospaced = true)
                }
            }
        }
    }
}

@Composable
private fun ProfileDebugRow(
    label: String,
    value: String,
    monospaced: Boolean = false,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = label,
            modifier = Modifier.weight(0.9f),
            style = MaterialTheme.typography.bodyMedium,
            color = IrisTheme.palette.muted,
        )
        Text(
            text = value,
            modifier = Modifier.weight(1.1f),
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.SemiBold,
            fontFamily = if (monospaced) FontFamily.Monospace else null,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

private fun lastHandshakeText(seconds: ULong?): String {
    val value = seconds?.toLong() ?: return "Never"
    val ageSecs = ((System.currentTimeMillis() / 1_000L) - value).coerceAtLeast(0L)
    return when {
        ageSecs < 60L -> "Just now"
        ageSecs < 3_600L -> "${ageSecs / 60L}m ago"
        ageSecs < 86_400L -> "${ageSecs / 3_600L}h ago"
        else -> "${ageSecs / 86_400L}d ago"
    }
}

@Composable
internal fun DisappearingMessagesCard(
    currentTtlSeconds: ULong?,
    onSelect: (ULong?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberIrisHapticFeedback()
    IrisSectionCard(modifier = modifier) {
        Text(
            text = "Disappearing messages",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text = "Messages auto-delete after the chosen interval.",
            style = MaterialTheme.typography.bodySmall,
            color = IrisTheme.palette.muted,
        )
        Column {
            DisappearingMessageOptions.forEach { (label, ttlSeconds) ->
                val interactionSource = remember(ttlSeconds) { MutableInteractionSource() }
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .clickable(
                                interactionSource = interactionSource,
                                indication = null,
                            ) {
                                haptics.press()
                                onSelect(ttlSeconds)
                            }
                            .padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    if (currentTtlSeconds == ttlSeconds) {
                        Icon(
                            imageVector = Icons.Rounded.Check,
                            contentDescription = "Selected",
                            tint = MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }
            }
        }
    }
}

private fun Modifier.clearFocusOnTapOutside(
    ignoredBounds: Rect?,
    onTapOutside: () -> Unit,
): Modifier =
    pointerInput(ignoredBounds, onTapOutside) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Final)
            val up = waitForUpOrCancellation(pass = PointerEventPass.Final)
            if (up != null && ignoredBounds?.contains(down.position) != true) {
                onTapOutside()
            }
        }
    }

/// Scoped message search bound to a single conversation. Reached via
/// the magnifying-glass icon in the chat header; mirrors the Signal
/// in-conversation search experience without forcing the user to
/// navigate back to the chat list.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InChatSearchSheet(
    appManager: AppManager,
    chatId: String,
    chatDisplayName: String,
    onDismiss: () -> Unit,
) {
    var query by remember(chatId) { mutableStateOf("") }
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }
    val trimmed = query.trim()
    var messageSearchLimit by remember(chatId, trimmed) { mutableStateOf(InitialMessageSearchLimit) }
    var results by remember(chatId) { mutableStateOf<SearchResultSnapshot?>(null) }

    LaunchedEffect(chatId, trimmed, messageSearchLimit) {
        results =
            if (trimmed.isEmpty()) {
                null
            } else {
                appManager.search(trimmed, scopeChatId = chatId, limit = messageSearchLimit)
            }
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier =
                Modifier
                    .fillMaxSize()
                    .testTag("inChatSearchSheet"),
            color = MaterialTheme.colorScheme.background,
        ) {
            Scaffold(
                containerColor = MaterialTheme.colorScheme.background,
                topBar = {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 8.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IconButton(onClick = onDismiss) {
                            Icon(
                                imageVector = Icons.Filled.Clear,
                                contentDescription = "Close search",
                                tint = MaterialTheme.colorScheme.onBackground,
                            )
                        }
                        TextField(
                            value = query,
                            onValueChange = { query = it },
                            modifier = Modifier
                                .weight(1f)
                                .focusRequester(focusRequester)
                                .testTag("inChatSearchField"),
                            placeholder = { Text("Search in $chatDisplayName") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                            shape = RoundedCornerShape(24.dp),
                            colors = irisTextFieldColors(containerColor = IrisTheme.palette.panelRaised),
                        )
                    }
                },
            ) { padding ->
                LazyColumn(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .background(MaterialTheme.colorScheme.background),
                ) {
                    val current = results?.takeIf { it.matchesSearchRequest(trimmed, scopeChatId = chatId) }
                    if (trimmed.isEmpty()) {
                        item {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 48.dp),
                                contentAlignment = Alignment.Center,
                            ) {
                                Text(
                                    text = "Type to search messages in this chat.",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = IrisTheme.palette.muted,
                                )
                            }
                        }
                    } else if (current == null || current.messages.isEmpty()) {
                        item {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 48.dp),
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
                        val nowMs = System.currentTimeMillis()
                        items(current.messages, key = { it.messageId }) { hit ->
                            IrisChatListRow(
                                title = hit.chatDisplayName,
                                isMuted = false,
                                isPinned = false,
                                preview = hit.body,
                                timeLabel = formatRelativeTime(hit.createdAtSecs.toLong(), nowMs),
                                imageUrl = null,
                                imageData = null,
                                unreadCount = 0L,
                                lastMessageMine = false,
                                lastDelivery = null,
                                onClick = {
                                    onDismiss()
                                    appManager.openChatAtMessage(hit.chatId, hit.messageId)
                                },
                            )
                        }
                        if (current.messages.size.toUInt() >= messageSearchLimit) {
                            item(key = "in-chat-search-more") {
                                IrisSearchViewMoreRow {
                                    messageSearchLimit = nextMessageSearchLimit(messageSearchLimit)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
