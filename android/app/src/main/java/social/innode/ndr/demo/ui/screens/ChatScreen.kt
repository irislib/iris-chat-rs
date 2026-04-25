package social.innode.ndr.demo.ui.screens

import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import android.util.Log
import android.webkit.WebView
import android.webkit.MimeTypeMap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Reply
import androidx.compose.material.icons.rounded.AddReaction
import androidx.compose.material.icons.rounded.Archive
import androidx.compose.material.icons.rounded.Audiotrack
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import social.innode.ndr.demo.core.AppManager
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.rust.AppState
import social.innode.ndr.demo.rust.ChatKind
import social.innode.ndr.demo.rust.ChatMessageSnapshot
import social.innode.ndr.demo.rust.DeliveryState
import social.innode.ndr.demo.rust.MessageAttachmentSnapshot
import social.innode.ndr.demo.rust.MessageReactionSnapshot
import social.innode.ndr.demo.rust.OutgoingAttachment
import social.innode.ndr.demo.rust.Screen
import social.innode.ndr.demo.ui.components.DeliveryGlyph
import social.innode.ndr.demo.ui.components.IrisIcons
import social.innode.ndr.demo.ui.components.IrisTopBar
import social.innode.ndr.demo.ui.components.formatMessageClock
import social.innode.ndr.demo.ui.components.formatTimelineDay
import social.innode.ndr.demo.ui.components.isSameTimelineDay
import social.innode.ndr.demo.ui.components.messageBubbleShape
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
    appState: AppState,
    chatId: String,
) {
    val context = LocalContext.current
    val chat = appState.currentChat?.takeIf { it.chatId == chatId }
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
    var chatMenuOpen by remember(chatId) { mutableStateOf(false) }
    val backUnreadCount =
        remember(appState.chatList, chatId) {
            appState.chatList
                .filter { it.chatId != chatId }
                .fold(0uL) { total, thread -> total + thread.unreadCount }
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

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        topBar = {
            IrisTopBar(
                title =
                    when {
                        chat?.kind == ChatKind.GROUP && chat.subtitle != null ->
                            "${chat.displayName} · ${chat.subtitle}"
                        else -> chat?.displayName ?: "Chat"
                    },
                onBack = {
                    appManager.dispatch(
                        AppAction.UpdateScreenStack(appState.router.screenStack.dropLast(1)),
                    )
                },
                backBadgeCount = backUnreadCount,
                actions = {
                    val groupId = chat?.groupId
                    if (chat != null) {
                        Box {
                            IconButton(
                                onClick = { chatMenuOpen = true },
                                modifier = Modifier.testTag("chatOverflowButton"),
                            ) {
                                Icon(
                                    imageVector = Icons.Rounded.MoreHoriz,
                                    contentDescription = "Chat options",
                                )
                            }
                            DropdownMenu(
                                expanded = chatMenuOpen,
                                onDismissRequest = { chatMenuOpen = false },
                            ) {
                                if (chat.kind == ChatKind.GROUP && groupId != null) {
                                    DropdownMenuItem(
                                        text = { Text("Group details") },
                                        onClick = {
                                            chatMenuOpen = false
                                            appManager.pushScreen(Screen.GroupDetails(groupId))
                                        },
                                    )
                                }
                                DisappearingMessageOptions.forEach { (label, ttlSeconds) ->
                                    DropdownMenuItem(
                                        text = {
                                            Text(
                                                if (chat.messageTtlSeconds == ttlSeconds) {
                                                    "✓ $label"
                                                } else {
                                                    label
                                                },
                                            )
                                        },
                                        onClick = {
                                            chatMenuOpen = false
                                            appManager.dispatch(
                                                AppAction.SetChatMessageTtl(chat.chatId, ttlSeconds),
                                            )
                                        },
                                    )
                                }
                            }
                        }
                    } else if (groupId != null) {
                        IconButton(onClick = { appManager.pushScreen(Screen.GroupDetails(groupId)) }) {
                            Icon(
                                imageVector = IrisIcons.Devices,
                                contentDescription = "Group details",
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
        val visibleMessages = chat.messages

        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .background(MaterialTheme.colorScheme.background),
        ) {
            Column(modifier = Modifier.fillMaxSize()) {
                LazyColumn(
                    state = listState,
                    modifier =
                        Modifier
                            .weight(1f)
                            .fillMaxWidth()
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
                    TypingIndicatorRow(
                        names = chat.typingIndicators.map { indicator -> indicator.displayName },
                    )
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
                    isSending = appState.busy.sendingMessage,
                    isUploading = appState.busy.uploadingAttachment,
                    onDraftChange = { value ->
                        draft = value
                        if (value.isNotBlank()) {
                            val nowMs = System.currentTimeMillis()
                            if (nowMs - lastTypingSentMs >= 3_000L) {
                                lastTypingSentMs = nowMs
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
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageBubble(
    message: ChatMessageSnapshot,
    chatKind: ChatKind,
    isFirstInCluster: Boolean,
    isLastInCluster: Boolean,
    reactions: List<MessageReactionSnapshot>,
    onReply: () -> Unit,
    onReact: (String) -> Unit,
    onDelete: () -> Unit,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, String) -> Unit,
) {
    val clipboard = rememberIrisClipboard()
    val parsed = remember(message.body) { parseReplyEncodedMessage(message.body) }
    val showActionDock = LocalConfiguration.current.screenWidthDp >= 600
    val hoverInteractionSource = remember { MutableInteractionSource() }
    val isHovering by hoverInteractionSource.collectIsHoveredAsState()
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .hoverable(hoverInteractionSource),
        horizontalArrangement = if (message.isOutgoing) Arrangement.End else Arrangement.Start,
    ) {
        Column(
            horizontalAlignment = if (message.isOutgoing) Alignment.End else Alignment.Start,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (showActionDock && isHovering && message.isOutgoing) {
                    MessageActionDock(
                        onReact = { onReact("❤️") },
                        onReply = onReply,
                        onInfo = {
                            clipboard.setText(
                                "Message info",
                                "Message ${message.id} · ${formatMessageClock(message.createdAtSecs.toLong())}",
                            )
                        },
                        onDelete = onDelete,
                    )
                }
                Surface(
                    modifier =
                        Modifier.combinedClickable(
                            onClick = {},
                            onLongClick = {
                                clipboard.setText("Message", copyableMessageText(message))
                            },
                        ),
                    color =
                        if (message.isOutgoing) {
                            IrisTheme.palette.bubbleMine
                        } else {
                            IrisTheme.palette.bubbleTheirs
                        },
                    shape =
                        messageBubbleShape(
                            isOutgoing = message.isOutgoing,
                            isFirstInCluster = isFirstInCluster,
                            isLastInCluster = isLastInCluster,
                        ),
                    tonalElevation = 0.dp,
                    shadowElevation = 0.dp,
                ) {
                    Column(
                        modifier =
                            Modifier
                                .padding(horizontal = 14.dp, vertical = 10.dp)
                                .testTag("chatMessage-${message.id}"),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        if (!message.isOutgoing && chatKind == ChatKind.GROUP && isFirstInCluster) {
                            Text(
                                text = message.author,
                                style = MaterialTheme.typography.labelMedium,
                                color = IrisTheme.palette.muted,
                            )
                        }
                        parsed.reply?.let { reply ->
                            ReplyPreview(reply = reply, isOutgoing = message.isOutgoing)
                        }
                        if (parsed.body.isNotBlank()) {
                            LinkedMessageText(
                                text = parsed.body,
                                style = MaterialTheme.typography.bodyLarge,
                                color =
                                    if (message.isOutgoing) {
                                        MaterialTheme.colorScheme.onPrimary
                                    } else {
                                        MaterialTheme.colorScheme.onSurface
                                    },
                                linkColor =
                                    if (message.isOutgoing) {
                                        MaterialTheme.colorScheme.onPrimary
                                    } else {
                                        IrisTheme.palette.accent
                                    },
                            )
                        }
                        message.attachments.forEach { attachment ->
                            AttachmentChip(
                                attachment = attachment,
                                isOutgoing = message.isOutgoing,
                                downloadAttachment = downloadAttachment,
                                onOpenImage = onOpenImage,
                            )
                        }
                        if (isLastInCluster) {
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                if (message.expiresAtSecs != null) {
                                    Icon(
                                        imageVector = Icons.Rounded.Schedule,
                                        contentDescription = "Disappearing message",
                                        tint =
                                            if (message.isOutgoing) {
                                                MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.72f)
                                            } else {
                                                IrisTheme.palette.muted
                                            },
                                        modifier =
                                            Modifier
                                                .size(13.dp)
                                                .testTag("chatMessageDisappearing-${message.id}"),
                                    )
                                }
                                Text(
                                    text = formatMessageClock(message.createdAtSecs.toLong()),
                                    style = MaterialTheme.typography.labelSmall,
                                    color =
                                        if (message.isOutgoing) {
                                            MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.72f)
                                        } else {
                                            IrisTheme.palette.muted
                                        },
                                )
                                if (message.isOutgoing) {
                                    DeliveryGlyph(message.delivery)
                                }
                            }
                        }
                    }
                }
                if (showActionDock && isHovering && !message.isOutgoing) {
                    MessageActionDock(
                        onReact = { onReact("❤️") },
                        onReply = onReply,
                        onInfo = {
                            clipboard.setText(
                                "Message info",
                                "Message ${message.id} · ${formatMessageClock(message.createdAtSecs.toLong())}",
                            )
                        },
                        onDelete = onDelete,
                    )
                }
            }
            if (reactions.isNotEmpty()) {
                ReactionRow(reactions = reactions)
            }
        }
    }
}

@Composable
private fun MessageActionDock(
    onReact: () -> Unit,
    onReply: () -> Unit,
    onInfo: () -> Unit,
    onDelete: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Surface(
        color = IrisTheme.palette.toolbar,
        shape = RoundedCornerShape(100.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 4.dp, vertical = 3.dp),
            horizontalArrangement = Arrangement.spacedBy(1.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ActionDockIconButton(Icons.Rounded.AddReaction, "React", onReact)
            ActionDockIconButton(Icons.AutoMirrored.Rounded.Reply, "Reply", onReply)
            Box {
                ActionDockIconButton(Icons.Rounded.MoreHoriz, "More", { menuOpen = true })
                DropdownMenu(
                    expanded = menuOpen,
                    onDismissRequest = { menuOpen = false },
                ) {
                    DropdownMenuItem(
                        text = { Text("Message info") },
                        onClick = {
                            menuOpen = false
                            onInfo()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text("Delete message") },
                        onClick = {
                            menuOpen = false
                            onDelete()
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun ActionDockIconButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    Box(
        modifier =
            Modifier
                .size(28.dp)
                .clip(CircleShape)
                .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = label,
            tint = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(18.dp),
        )
    }
}

@Composable
private fun ReplyPreview(
    reply: ReplyPreviewData,
    isOutgoing: Boolean,
) {
    Surface(
        color =
            if (isOutgoing) {
                MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.12f)
            } else {
                MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
            },
        shape = RoundedCornerShape(10.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                modifier =
                    Modifier
                        .size(width = 3.dp, height = 34.dp)
                        .clip(CircleShape)
                        .background(if (isOutgoing) MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.6f) else IrisTheme.palette.accent),
            )
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = reply.author,
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = reply.body,
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun ReactionRow(reactions: List<MessageReactionSnapshot>) {
    Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
        reactions.forEach { reaction ->
            Surface(
                color =
                    if (reaction.reactedByMe) {
                        IrisTheme.palette.accent.copy(alpha = 0.18f)
                    } else {
                        IrisTheme.palette.panel
                    },
                shape = RoundedCornerShape(100.dp),
            ) {
                Text(
                    text = "${reaction.emoji} ${reaction.count}",
                    modifier = Modifier.padding(horizontal = 7.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }
}

@Composable
private fun TypingIndicatorRow(names: List<String>) {
    val label =
        when {
            names.isEmpty() -> ""
            names.size == 1 -> "${names.first()} is typing"
            else -> "${names.first()} and ${names.size - 1} more are typing"
        }
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("chatTypingIndicator"),
        color = IrisTheme.palette.toolbar,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                repeat(3) {
                    Box(
                        modifier =
                            Modifier
                                .size(5.dp)
                                .clip(CircleShape)
                                .background(IrisTheme.palette.muted),
                    )
                }
            }
            Text(
                text = label,
                style = MaterialTheme.typography.labelMedium,
                color = IrisTheme.palette.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun ReplyComposerStrip(
    message: ChatMessageSnapshot,
    onCancel: () -> Unit,
) {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("chatReplyComposer"),
        color = IrisTheme.palette.toolbar,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier =
                    Modifier
                        .size(width = 3.dp, height = 38.dp)
                        .clip(CircleShape)
                        .background(IrisTheme.palette.accent),
            )
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = message.author,
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = replySnippet(message),
                    style = MaterialTheme.typography.labelSmall,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(onClick = onCancel) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Cancel reply",
                    tint = IrisTheme.palette.muted,
                )
            }
        }
    }
}

private data class ReplyPreviewData(
    val author: String,
    val body: String,
)

private data class ParsedReplyMessage(
    val reply: ReplyPreviewData?,
    val body: String,
)

private fun replyEncodedMessage(
    reply: ChatMessageSnapshot?,
    text: String,
): String {
    if (reply == null) {
        return text
    }
    return "$ReplyMessagePrefix${reply.author}: ${replySnippet(reply)}\n\n$text"
}

private fun parseReplyEncodedMessage(text: String): ParsedReplyMessage {
    if (!text.startsWith(ReplyMessagePrefix)) {
        return ParsedReplyMessage(reply = null, body = text)
    }
    val remaining = text.removePrefix(ReplyMessagePrefix)
    val separator = remaining.indexOf("\n\n")
    if (separator < 0) {
        return ParsedReplyMessage(reply = null, body = text)
    }
    val header = remaining.substring(0, separator)
    val body = remaining.substring(separator + 2)
    val splitAt = header.indexOf(':')
    if (splitAt <= 0) {
        return ParsedReplyMessage(reply = null, body = text)
    }
    return ParsedReplyMessage(
        reply =
            ReplyPreviewData(
                author = header.substring(0, splitAt).trim(),
                body = header.substring(splitAt + 1).trim(),
            ),
        body = body,
    )
}

private fun replySnippet(message: ChatMessageSnapshot): String {
    val parsed = parseReplyEncodedMessage(message.body)
    val source = parsed.body.ifBlank { copyableMessageText(message) }
    val normalized = source.replace('\n', ' ').trim()
    if (normalized.isBlank()) {
        return message.attachments.firstOrNull()?.filename ?: "Attachment"
    }
    return normalized.take(96)
}

private const val ReplyMessagePrefix = "↩ "

@Composable
private fun LinkedMessageText(
    text: String,
    style: TextStyle,
    color: Color,
    linkColor: Color,
) {
    val annotated = remember(text, linkColor) {
        linkedMessageAnnotatedString(text, linkColor)
    }

    Text(
        text = annotated,
        style = style.copy(color = color),
    )
}

private fun linkedMessageAnnotatedString(
    text: String,
    linkColor: Color,
): AnnotatedString =
    buildAnnotatedString {
        var index = 0
        for (match in MessageUrlRegex.findAll(text)) {
            val range = trimTrailingUrlPunctuation(match.value)
            if (range.isEmpty()) {
                continue
            }
            append(text.substring(index, match.range.first))
            val visible = range
            val url = normalizedMessageUrl(visible)
            val start = length
            append(visible)
            addLink(
                LinkAnnotation.Url(
                    url = url,
                    styles = TextLinkStyles(style = SpanStyle(color = linkColor)),
                ),
                start,
                length,
            )
            index = match.range.first + visible.length
        }
        if (index < text.length) {
            append(text.substring(index))
        }
    }

private fun trimTrailingUrlPunctuation(value: String): String =
    value.trimEnd('.', ',', ';', ':', '!', '?', ')', ']')

private fun normalizedMessageUrl(value: String): String =
    if (value.startsWith("http://", ignoreCase = true) ||
        value.startsWith("https://", ignoreCase = true)
    ) {
        value
    } else {
        "https://$value"
    }

private val MessageUrlRegex = Regex("""(?i)\b((https?://|www\.)[^\s<]+)""")

@Composable
private fun ComposerBar(
    draft: String,
    selectedAttachments: List<PickedAttachment>,
    isSending: Boolean,
    isUploading: Boolean,
    onDraftChange: (String) -> Unit,
    onAttach: () -> Unit,
    onRemoveAttachment: (PickedAttachment) -> Unit,
    onSend: () -> Unit,
) {
    val isBusy = isSending || isUploading
    val canSend = (draft.isNotBlank() || selectedAttachments.isNotEmpty()) && !isBusy
    val showDesktopComposerTools = LocalConfiguration.current.screenWidthDp >= 600
    var showingEmojiPicker by remember { mutableStateOf(false) }
    fun submitDraft() {
        if (canSend) {
            onSend()
        }
    }

    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .imePadding(),
        color = IrisTheme.palette.toolbar,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 14.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (selectedAttachments.isNotEmpty()) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .horizontalScroll(rememberScrollState())
                            .testTag("chatSelectedAttachments"),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    selectedAttachments.forEach { attachment ->
                        SelectedAttachmentChip(
                            attachment = attachment,
                            enabled = !isBusy,
                            onRemove = { onRemoveAttachment(attachment) },
                        )
                    }
                }
            }

            if (isUploading) {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(5.dp),
                ) {
                    Text(
                        text = "Uploading attachment",
                        style = MaterialTheme.typography.labelMedium,
                        color = IrisTheme.palette.muted,
                    )
                    LinearProgressIndicator(
                        modifier = Modifier.fillMaxWidth(),
                        color = IrisTheme.palette.accent,
                        trackColor = IrisTheme.palette.muted.copy(alpha = 0.18f),
                    )
                }
            }

            if (showDesktopComposerTools && showingEmojiPicker) {
                EmojiPickerRow(
                    enabled = !isBusy,
                    onEmoji = { emoji ->
                        onDraftChange(draft + emoji)
                        showingEmojiPicker = false
                    },
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.Bottom,
            ) {
                IconButton(
                    onClick = onAttach,
                    enabled = !isBusy,
                    modifier =
                        Modifier
                            .size(48.dp)
                            .testTag("chatAttachButton"),
                ) {
                    if (isUploading) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp,
                            color = IrisTheme.palette.muted,
                        )
                    } else {
                        Icon(
                            imageVector = IrisIcons.Attach,
                            contentDescription = "Attach",
                            tint =
                                if (isBusy) {
                                    IrisTheme.palette.muted.copy(alpha = 0.54f)
                                } else {
                                    MaterialTheme.colorScheme.onSurface
                                },
                        )
                    }
                }

                if (showDesktopComposerTools) {
                    IconButton(
                        onClick = { showingEmojiPicker = !showingEmojiPicker },
                        enabled = !isBusy,
                        modifier =
                            Modifier
                                .size(48.dp)
                                .testTag("chatEmojiButton"),
                    ) {
                        Text(
                            text = "☺",
                            style = MaterialTheme.typography.titleLarge,
                            color =
                                if (isBusy) {
                                    IrisTheme.palette.muted.copy(alpha = 0.54f)
                                } else {
                                    MaterialTheme.colorScheme.onSurface
                                },
                        )
                    }
                }

                Surface(
                    modifier = Modifier.weight(1f),
                    color = IrisTheme.palette.panel,
                    shape = RoundedCornerShape(24.dp),
                ) {
                    TextField(
                        value = draft,
                        onValueChange = onDraftChange,
                        placeholder = {
                            Text(
                                text = "Message",
                                color = IrisTheme.palette.muted,
                            )
                        },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("chatMessageInput"),
                        minLines = 1,
                        maxLines = 5,
                        colors =
                            TextFieldDefaults.colors(
                                focusedContainerColor = Color.Transparent,
                                unfocusedContainerColor = Color.Transparent,
                                disabledContainerColor = Color.Transparent,
                                focusedIndicatorColor = Color.Transparent,
                                unfocusedIndicatorColor = Color.Transparent,
                                disabledIndicatorColor = Color.Transparent,
                            ),
                    )
                }

                Surface(
                    modifier =
                        Modifier
                            .size(52.dp)
                            .clip(CircleShape),
                    color = IrisTheme.palette.accent,
                    shape = CircleShape,
                ) {
                    IconButton(
                        onClick = { submitDraft() },
                        enabled = canSend,
                        modifier = Modifier.testTag("chatSendButton"),
                    ) {
                        if (isSending) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onPrimary,
                            )
                        } else {
                            Icon(
                                imageVector = IrisIcons.Send,
                                contentDescription = "Send",
                                tint = MaterialTheme.colorScheme.onPrimary,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun EmojiPickerRow(
    enabled: Boolean,
    onEmoji: (String) -> Unit,
) {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("chatEmojiPicker"),
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(18.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 8.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ComposerEmojiChoices.forEach { emoji ->
                Box(
                    modifier =
                        Modifier
                            .size(36.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(enabled = enabled) { onEmoji(emoji) },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = emoji,
                        style = MaterialTheme.typography.titleMedium,
                    )
                }
            }
        }
    }
}

private val ComposerEmojiChoices =
    listOf(
        "😀", "😂", "😊", "😍", "🥰", "😎", "🤔", "😭",
        "❤️", "🔥", "✨", "🙏", "👍", "👀", "🎉", "💜",
        "🌞", "🌙", "⭐️", "🍓", "☕️", "🌊", "🚀", "✅",
    )

@Composable
private fun SelectedAttachmentChip(
    attachment: PickedAttachment,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    val selectedAttachmentType = attachmentType(attachment)

    Surface(
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.semantics {
            contentDescription = "${selectedAttachmentType.label}, ${attachment.filename}"
        },
    ) {
        Row(
            modifier = Modifier.padding(start = 10.dp, top = 7.dp, end = 4.dp, bottom = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = selectedAttachmentType.icon,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(18.dp),
            )
            Column(
                modifier = Modifier.widthIn(max = 200.dp),
                verticalArrangement = Arrangement.spacedBy(1.dp),
            ) {
                Text(
                    text = attachment.filename,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = selectedAttachmentType.label,
                    style = MaterialTheme.typography.labelSmall,
                    color = IrisTheme.palette.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(
                onClick = onRemove,
                enabled = enabled,
                modifier =
                    Modifier
                        .size(28.dp)
                        .testTag("chatSelectedAttachmentRemove"),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Remove attachment",
                    tint = IrisTheme.palette.muted,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

private data class PickedAttachment(
    val path: String,
    val filename: String,
)

private enum class ChatAttachmentType(
    val label: String,
    val icon: ImageVector,
) {
    IMAGE("Image", Icons.Rounded.Image),
    VIDEO("Video", Icons.Rounded.Movie),
    AUDIO("Audio", Icons.Rounded.Audiotrack),
    ARCHIVE("Archive", Icons.Rounded.Archive),
    DOCUMENT("Document", Icons.Rounded.Description),
    FILE("File", IrisIcons.File),
}

private val chatImageExtensions = setOf(
    "gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif",
)
private val chatVideoExtensions = setOf("avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts")
private val chatAudioExtensions = setOf("aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma")
private val chatArchiveExtensions = setOf("7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip")
private val chatDocumentExtensions = setOf(
    "csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml",
)

private fun attachmentType(attachment: PickedAttachment): ChatAttachmentType =
    attachmentType(attachment.filename)

private fun attachmentType(attachment: MessageAttachmentSnapshot): ChatAttachmentType {
    if (attachment.isImage) {
        return ChatAttachmentType.IMAGE
    }
    if (attachment.isVideo) {
        return ChatAttachmentType.VIDEO
    }
    if (attachment.isAudio) {
        return ChatAttachmentType.AUDIO
    }
    return attachmentType(attachment.filename)
}

private fun attachmentType(filename: String): ChatAttachmentType {
    val extension = filename.substringAfterLast(".", "").trim().lowercase()
    if (extension.isEmpty()) {
        return ChatAttachmentType.FILE
    }
    if (chatImageExtensions.contains(extension)) {
        return ChatAttachmentType.IMAGE
    }
    if (chatVideoExtensions.contains(extension)) {
        return ChatAttachmentType.VIDEO
    }
    if (chatAudioExtensions.contains(extension)) {
        return ChatAttachmentType.AUDIO
    }
    if (chatArchiveExtensions.contains(extension)) {
        return ChatAttachmentType.ARCHIVE
    }
    if (chatDocumentExtensions.contains(extension)) {
        return ChatAttachmentType.DOCUMENT
    }
    return ChatAttachmentType.FILE
}

private fun copyAttachmentToCache(
    context: Context,
    uri: Uri,
): PickedAttachment? {
    val resolver = context.contentResolver
    val displayName = displayNameForUri(context, uri)
    val outputDir = File(context.cacheDir, "attachments/outgoing").apply { mkdirs() }
    val outputFile = File(outputDir, "${UUID.randomUUID()}-$displayName")

    return runCatching {
        resolver.openInputStream(uri)?.use { input ->
            outputFile.outputStream().use { output ->
                input.copyTo(output)
            }
        } ?: return null
        PickedAttachment(outputFile.absolutePath, displayName)
    }.onFailure { error ->
        Log.w(ChatScreenLogTag, "failed to copy attachment", error)
    }.getOrNull()
}

private fun displayNameForUri(
    context: Context,
    uri: Uri,
): String {
    val queried =
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
            }
    return safeAttachmentName(queried ?: uri.lastPathSegment ?: "attachment")
}

private fun safeAttachmentName(value: String): String {
    val basename = value.substringAfterLast('/').substringAfterLast('\\').trim()
    return basename.ifEmpty { "attachment" }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AttachmentChip(
    attachment: MessageAttachmentSnapshot,
    isOutgoing: Boolean,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, String) -> Unit,
) {
    val context = LocalContext.current
    val clipboard = rememberIrisClipboard()
    val scope = rememberCoroutineScope()
    var localImageData by remember(attachment.htreeUrl) { mutableStateOf<ByteArray?>(null) }
    var imageLoadFailed by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var imageLoading by remember(attachment.htreeUrl) { mutableStateOf(false) }
    var attachmentOpening by remember(attachment.htreeUrl) { mutableStateOf(false) }
    val foreground =
        if (isOutgoing) {
            MaterialTheme.colorScheme.onPrimary
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    val type = attachmentType(attachment)

    suspend fun loadImageIfNeeded(): ByteArray? {
        localImageData?.let { return it }
        if (!attachment.isImage || imageLoading) {
            return null
        }
        imageLoading = true
        imageLoadFailed = false
        val data = downloadAttachment(attachment)
        imageLoading = false
        if (data == null) {
            imageLoadFailed = true
            return null
        }
        localImageData = data
        return data
    }

    if (attachment.isImage) {
        LaunchedEffect(attachment.htreeUrl) {
            loadImageIfNeeded()
        }
        val isAnimated = remember(localImageData, attachment.filename) {
            localImageData?.let { data -> isAnimatedImage(data, attachment.filename) } ?: isLikelyGif(attachment.filename)
        }
        val bitmap = remember(localImageData) {
            localImageData
                ?.takeUnless { data -> isAnimatedImage(data, attachment.filename) }
                ?.let { data -> BitmapFactory.decodeByteArray(data, 0, data.size) }
        }
        Column(
            modifier =
                Modifier
                    .clip(RoundedCornerShape(16.dp))
                    .clickable {
                        val data = localImageData
                        if (data != null) {
                            onOpenImage(data, attachment.filename)
                        } else {
                            scope.launch {
                                loadImageIfNeeded()?.let { loadedData ->
                                    onOpenImage(loadedData, attachment.filename)
                                }
                            }
                        }
                    },
            verticalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            Box(
                modifier =
                    Modifier
                        .size(width = 220.dp, height = 150.dp)
                        .clip(RoundedCornerShape(16.dp))
                        .background(foreground.copy(alpha = 0.12f)),
                contentAlignment = Alignment.Center,
            ) {
                if (bitmap != null) {
                    Image(
                        bitmap = bitmap.asImageBitmap(),
                        contentDescription = attachment.filename,
                        modifier = Modifier.fillMaxSize(),
                        contentScale = ContentScale.Crop,
                    )
                } else if (isAnimated && localImageData != null) {
                    AnimatedImageDataView(
                        data = localImageData!!,
                        modifier = Modifier.fillMaxSize(),
                    )
                } else if (imageLoading) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(22.dp),
                        strokeWidth = 2.dp,
                        color = foreground,
                    )
                } else {
                    Icon(
                        imageVector = if (imageLoadFailed) Icons.Rounded.Warning else IrisIcons.Image,
                        contentDescription = null,
                        tint = foreground.copy(alpha = 0.72f),
                        modifier = Modifier.size(30.dp),
                    )
                }
            }
            Text(
                text = attachment.filename,
                modifier = Modifier.widthIn(max = 220.dp),
                style = MaterialTheme.typography.labelSmall,
                color = foreground,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        return
    }

    Row(
        modifier =
            Modifier
                .semantics { contentDescription = "${type.label}, ${attachment.filename}" }
                .clip(RoundedCornerShape(12.dp))
                .background(foreground.copy(alpha = 0.12f))
                .combinedClickable(
                    onClick = {
                        if (attachmentOpening) {
                            return@combinedClickable
                        }
                        scope.launch {
                            attachmentOpening = true
                            val data = downloadAttachment(attachment)
                            val opened = data?.let {
                                openDownloadedAttachment(context, attachment, it)
                            } ?: false
                            attachmentOpening = false
                            if (!opened) {
                                clipboard.setText(attachment.filename, attachment.htreeUrl)
                            }
                        }
                    },
                    onLongClick = {
                        clipboard.setText(attachment.filename, attachment.htreeUrl)
                    },
                )
                .padding(horizontal = 10.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (attachmentOpening) {
            CircularProgressIndicator(
                modifier = Modifier.size(20.dp),
                strokeWidth = 2.dp,
                color = foreground,
            )
        } else {
            Icon(
                imageVector = type.icon,
                contentDescription = null,
                tint = foreground,
                modifier = Modifier.size(20.dp),
            )
        }
        Column(
            modifier = Modifier.widthIn(max = 220.dp),
            verticalArrangement = Arrangement.spacedBy(1.dp),
        ) {
            Text(
                text = attachment.filename,
                style = MaterialTheme.typography.labelLarge,
                color = foreground,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = type.label,
                style = MaterialTheme.typography.labelSmall,
                color = foreground.copy(alpha = 0.72f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun openDownloadedAttachment(
    context: Context,
    attachment: MessageAttachmentSnapshot,
    data: ByteArray,
): Boolean =
    runCatching {
        val outputDir = File(context.cacheDir, "attachments/downloaded").apply { mkdirs() }
        val outputFile = File(outputDir, attachmentCacheName(attachment.nhash, attachment.filename))
        outputFile.writeBytes(data)
        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                outputFile,
            )
        val intent =
            Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, mimeTypeForFilename(attachment.filename))
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        context.startActivity(Intent.createChooser(intent, attachment.filename))
        true
    }.onFailure { error ->
        Log.w(ChatScreenLogTag, "failed to open attachment", error)
    }.getOrDefault(false)

private fun attachmentCacheName(
    nhash: String,
    filename: String,
): String = "${safeAttachmentCacheComponent(nhash)}-${safeAttachmentCacheComponent(filename)}"

private fun safeAttachmentCacheComponent(value: String): String =
    value
        .split('/', '\\', ':')
        .joinToString("-")
        .trim()
        .ifEmpty { "attachment" }

private fun mimeTypeForFilename(filename: String): String {
    val extension = filename.substringAfterLast('.', "").lowercase()
    return extension
        .takeIf { it.isNotBlank() }
        ?.let { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) }
        ?: "application/octet-stream"
}

private data class DownloadedImageAttachment(
    val data: ByteArray,
    val filename: String,
)

@Composable
private fun ImageViewerDialog(
    item: DownloadedImageAttachment,
    onDismiss: () -> Unit,
) {
    val focusRequester = remember { FocusRequester() }
    val bitmap = remember(item.data) {
        item.data
            .takeUnless { data -> isAnimatedImage(data, item.filename) }
            ?.let { data -> BitmapFactory.decodeByteArray(data, 0, data.size) }
    }
    val isAnimated = remember(item.data, item.filename) {
        isAnimatedImage(item.data, item.filename)
    }
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.92f))
                    .clickable(onClick = onDismiss)
                    .focusRequester(focusRequester)
                    .focusable()
                    .onPreviewKeyEvent { event ->
                        if (event.key == Key.Escape && event.type == KeyEventType.KeyUp) {
                            onDismiss()
                            true
                        } else {
                            false
                        }
                    },
            contentAlignment = Alignment.Center,
        ) {
            if (isAnimated) {
                AnimatedImageDataView(
                    data = item.data,
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                )
            } else if (bitmap != null) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = item.filename,
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(18.dp),
                    contentScale = ContentScale.Fit,
                )
            } else {
                CircularProgressIndicator(color = Color.White)
            }
            IconButton(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.TopEnd),
            ) {
                Icon(
                    imageVector = IrisIcons.Close,
                    contentDescription = "Close image",
                    tint = Color.White,
                )
            }
        }
    }
}

@Composable
private fun AnimatedImageDataView(
    data: ByteArray,
    modifier: Modifier = Modifier,
) {
    val html = remember(data) { animatedImageHtml(data) }
    AndroidView(
        modifier = modifier,
        factory = { context ->
            WebView(context).apply {
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
                settings.javaScriptEnabled = false
                isVerticalScrollBarEnabled = false
                isHorizontalScrollBarEnabled = false
                loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
            }
        },
        update = { webView ->
            webView.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
        },
    )
}

private fun animatedImageHtml(data: ByteArray): String {
    val encoded = Base64.encodeToString(data, Base64.NO_WRAP)
    return """
        <!doctype html>
        <html>
        <head>
        <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1">
        <style>
        html, body {
          margin: 0;
          width: 100%;
          height: 100%;
          overflow: hidden;
          background: transparent;
        }
        body {
          display: flex;
          align-items: center;
          justify-content: center;
        }
        img {
          width: 100%;
          height: 100%;
          object-fit: contain;
        }
        </style>
        </head>
        <body><img src="data:image/gif;base64,$encoded" alt=""></body>
        </html>
    """.trimIndent()
}

private fun isLikelyGif(filename: String): Boolean =
    filename.endsWith(".gif", ignoreCase = true)

private fun isAnimatedImage(
    data: ByteArray,
    filename: String,
): Boolean =
    isLikelyGif(filename) ||
        data.take(6).toByteArray().contentEquals("GIF87a".toByteArray()) ||
        data.take(6).toByteArray().contentEquals("GIF89a".toByteArray())

private fun copyableMessageText(message: ChatMessageSnapshot): String {
    val pieces = buildList {
        if (message.body.isNotBlank()) {
            add(message.body)
        }
        message.attachments.forEach { attachment ->
            add(attachment.htreeUrl)
        }
    }
    return pieces.joinToString("\n")
}

private const val MessageClusterGapSecs = 60L
private const val ChatScreenLogTag = "IrisChat"

private fun startsMessageCluster(
    previous: ChatMessageSnapshot?,
    message: ChatMessageSnapshot,
    chatKind: ChatKind,
): Boolean {
    if (previous == null) {
        return true
    }
    val previousSecs = previous.createdAtSecs.toLong()
    val messageSecs = message.createdAtSecs.toLong()
    if (!isSameTimelineDay(previousSecs, messageSecs)) {
        return true
    }
    if (previous.isOutgoing != message.isOutgoing) {
        return true
    }
    if (chatKind == ChatKind.GROUP && !message.isOutgoing && previous.author != message.author) {
        return true
    }
    val gap = if (messageSecs >= previousSecs) messageSecs - previousSecs else 0
    if (gap <= MessageClusterGapSecs) {
        return false
    }
    if (chatKind == ChatKind.DIRECT) {
        val previousMinute = previousSecs / 60L
        val messageMinute = messageSecs / 60L
        if (messageMinute - previousMinute in 0L..1L) {
            return false
        }
    }
    return true
}
