package social.innode.ndr.demo.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
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
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInParent
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.ChatKind
import social.innode.ndr.demo.rust.ChatMessageSnapshot
import social.innode.ndr.demo.rust.DeliveryState
import social.innode.ndr.demo.rust.OutgoingAttachment
import social.innode.ndr.demo.rust.Screen
import social.innode.ndr.demo.rust.peerInputToNpub
import social.innode.ndr.demo.rust.proxiedImageUrl
import social.innode.ndr.demo.ui.components.IrisAvatar
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisInlineAction
import social.innode.ndr.demo.ui.components.IrisSectionCard
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.formatTimelineDay
import social.innode.ndr.demo.ui.components.isSameTimelineDay
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

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

@Composable
fun ChatScreen(
    appManager: AppManager,
    chatId: String,
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

    val context = LocalContext.current
    val focusManager = LocalFocusManager.current
    val chat = currentChat?.takeIf { it.chatId == chatId }
    var draft by remember(chatId) { mutableStateOf("") }
    var selectedAttachments by remember(chatId) { mutableStateOf<List<PickedAttachment>>(emptyList()) }
    val listState = rememberLazyListState()
    val coroutineScope = rememberCoroutineScope()
    var shouldFollowLatest by remember(chatId) { mutableStateOf(true) }
    var forceScrollToLatest by remember(chatId) { mutableStateOf(false) }
    var initialScrollPending by remember(chatId) { mutableStateOf(true) }
    var observedMessageCount by remember(chatId) { mutableStateOf(0) }
    var replyTarget by remember(chatId) { mutableStateOf<ChatMessageSnapshot?>(null) }
    var imageViewerItem by remember(chatId) { mutableStateOf<DownloadedImageAttachment?>(null) }
    var lastTypingSentMs by remember(chatId) { mutableStateOf(0L) }
    var hasSentTyping by remember(chatId) { mutableStateOf(false) }
    var directChatInfoOpen by remember(chatId) { mutableStateOf(false) }
    var composerBounds by remember { mutableStateOf<Rect?>(null) }
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

    LaunchedEffect(chatId, chat?.messages?.size, chat?.messages?.lastOrNull()?.id, forceScrollToLatest) {
        val total = chat?.messages?.size ?: 0
        if (total == 0) {
            initialScrollPending = true
            observedMessageCount = 0
            forceScrollToLatest = false
            return@LaunchedEffect
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

    val unseenIncomingIds =
        remember(chat?.messages) {
            chat
                ?.messages
                ?.filter { message -> !message.isOutgoing && message.delivery != DeliveryState.SEEN }
                ?.map { message -> message.id }
                .orEmpty()
        }

    LaunchedEffect(chatId, unseenIncomingIds) {
        if (unseenIncomingIds.isNotEmpty()) {
            appManager.dispatch(AppAction.MarkMessagesSeen(chatId, unseenIncomingIds))
        }
    }

    DisposableEffect(chatId) {
        onDispose {
            if (hasSentTyping) {
                hasSentTyping = false
                lastTypingSentMs = 0L
                appManager.dispatch(AppAction.StopTyping(chatId))
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
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(router.screenStack.dropLast(1)),
                    )
                },
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
                actions = {},
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
        val visibleMessages = chat.messages

        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background)
                    .clearFocusOnTapOutside(composerBounds) {
                        focusManager.clearFocus()
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
                                .testTag("chatTimeline")
                                .padding(horizontal = 14.dp),
                        verticalArrangement = Arrangement.spacedBy(2.dp),
                    ) {
                        itemsIndexed(visibleMessages, key = { _, message -> message.id }) { index, message ->
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
                                Box(
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .padding(vertical = 14.dp),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Surface(
                                        color = IrisTheme.palette.panel.copy(alpha = 0.58f),
                                        shape = RoundedCornerShape(100.dp),
                                    ) {
                                        Text(
                                            text = formatTimelineDay(message.createdAtSecs.toLong()),
                                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                            style = MaterialTheme.typography.labelMedium,
                                            color = IrisTheme.palette.muted,
                                        )
                                    }
                                }
                            }

                            MessageBubble(
                                message = message,
                                chatKind = chat.kind,
                                isFirstInCluster = isFirstInCluster,
                                isLastInCluster = isLastInCluster,
                                reactions = message.reactions,
                                onReply = { replyTarget = message },
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
                                downloadAttachment = { attachment ->
                                    appManager.downloadAttachment(attachment)
                                },
                                onOpenImage = { data, filename ->
                                    imageViewerItem = DownloadedImageAttachment(data = data, filename = filename)
                                },
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
                        if (hasSentTyping) {
                            hasSentTyping = false
                            lastTypingSentMs = 0L
                            appManager.dispatch(AppAction.StopTyping(chatId))
                        }
                    },
                )
            }

            if (showJumpToBottom) {
                Surface(
                    modifier =
                        Modifier
                            .align(Alignment.BottomEnd)
                            .padding(end = 18.dp, bottom = 104.dp)
                            .testTag("chatJumpToBottom"),
                    color = IrisTheme.palette.panel,
                    shape = CircleShape,
                    shadowElevation = 0.dp,
                ) {
                    IconButton(
                        onClick = {
                            shouldFollowLatest = true
                            coroutineScope.launch {
                                val total = chat.messages.size
                                if (total > 0) {
                                    listState.animateScrollToItem(total - 1)
                                }
                            }
                        },
                    ) {
                        Text(
                            text = "↓",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                }
            }

            imageViewerItem?.let { item ->
                ImageViewerDialog(
                    item = item,
                    onDismiss = { imageViewerItem = null },
                )
            }

            if (directChatInfoOpen) {
                DirectChatInfoSheet(
                    appManager = appManager,
                    chatId = chatId,
                    onDismiss = { directChatInfoOpen = false },
                )
            }
        }
    }
}

@Composable
private fun DirectChatInfoSheet(
    appManager: AppManager,
    chatId: String,
    onDismiss: () -> Unit,
) {
    val currentChat by appManager.currentChat.collectAsStateWithLifecycle()
    val preferences by appManager.preferences.collectAsStateWithLifecycle()
    val chat = currentChat?.takeIf { it.chatId == chatId } ?: return
    val avatarBytes by rememberNhashImageData(appManager, chat.pictureUrl)
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

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier =
                Modifier
                    .fillMaxSize()
                    .testTag("directChatInfoSheet"),
            color = MaterialTheme.colorScheme.background,
        ) {
            Scaffold(
                containerColor = MaterialTheme.colorScheme.background,
                topBar = {
                    IrisTopBar(
                        title = chat.displayName,
                        onBack = onDismiss,
                    )
                },
            ) { padding ->
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(padding)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    IrisAvatar(
                        label = chat.displayName,
                        size = 96.dp,
                        emphasize = true,
                        imageUrl = proxiedAvatarUrl,
                        imageData = avatarBytes,
                    )
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
                    val clipboard = rememberIrisClipboard()
                    IrisInlineAction(
                        text = "Copy user ID",
                        onClick = { clipboard.setText("User ID", peerInputToNpub(chatId)) },
                        modifier = Modifier.testTag("directChatCopyUserIdButton"),
                    ) {
                        Icon(imageVector = IrisIcons.Copy, contentDescription = null)
                    }
                    DisappearingMessagesCard(
                        currentTtlSeconds = chat.messageTtlSeconds,
                        onSelect = { ttlSeconds ->
                            appManager.dispatch(AppAction.SetChatMessageTtl(chatId, ttlSeconds))
                        },
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }
    }
}

@Composable
internal fun DisappearingMessagesCard(
    currentTtlSeconds: ULong?,
    onSelect: (ULong?) -> Unit,
    modifier: Modifier = Modifier,
) {
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
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(ttlSeconds) }
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
                            tint = IrisTheme.palette.accent,
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
