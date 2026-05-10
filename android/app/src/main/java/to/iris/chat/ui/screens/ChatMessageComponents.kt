package to.iris.chat.ui.screens

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Reply
import androidx.compose.material.icons.rounded.AddReaction
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.IntOffset
import kotlinx.coroutines.launch
import kotlin.math.abs
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.rust.MessageReactionSnapshot
import to.iris.chat.rust.MessageReactor
import to.iris.chat.rust.MessageRecipientDeliverySnapshot
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AccountSnapshot
import to.iris.chat.rust.peerInputToNpub
import to.iris.chat.ui.components.DeliveryGlyph
import to.iris.chat.ui.components.IrisAvatar
import to.iris.chat.ui.components.IrisEmojiPickerSheet
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.formatMessageClock
import to.iris.chat.ui.components.irisReactionQuickChoices
import to.iris.chat.ui.components.isSameTimelineDay
import to.iris.chat.ui.components.loadRecentReactionEmojis
import to.iris.chat.ui.components.messageBubbleShape
import to.iris.chat.ui.components.rememberRecentReactionEmoji
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.ui.theme.IrisTheme
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.TextButton
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun MessageBubble(
    message: ChatMessageSnapshot,
    chatKind: ChatKind,
    isFirstInCluster: Boolean,
    isLastInCluster: Boolean,
    reactions: List<MessageReactionSnapshot>,
    onReply: () -> Unit,
    onReact: (String) -> Unit,
    onDelete: () -> Unit,
    onScrollToQuote: () -> Unit,
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, String) -> Unit,
    chat: CurrentChatSnapshot? = null,
    appManager: AppManager? = null,
) {
    if (message.kind == ChatMessageKind.SYSTEM) {
        SystemMessageChip(message = message)
        return
    }

    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current.applicationContext
    val hapticFeedback = LocalHapticFeedback.current
    val parsed = remember(message.body) { parseReplyEncodedMessage(message.body) }
    val postReactionSuggestions = remember(reactions) { postReactionSuggestionEmojis(reactions) }
    fun pickReaction(emoji: String) {
        rememberRecentReactionEmoji(context, emoji)
        onReact(emoji)
    }
    val showDesktopActionDock = LocalConfiguration.current.screenWidthDp >= 600
    val hoverInteractionSource = remember { MutableInteractionSource() }
    val isHovering by hoverInteractionSource.collectIsHoveredAsState()
    val showActionDock = showDesktopActionDock && isHovering
    var isInfoOpen by remember(message.id) { mutableStateOf(false) }
    var isActionsSheetOpen by remember(message.id) { mutableStateOf(false) }
    var isReactionPickerOpen by remember(message.id) { mutableStateOf(false) }
    var isReactorsSheetOpen by remember(message.id) { mutableStateOf(false) }
    if (isReactorsSheetOpen) {
        MessageReactorsSheet(
            reactors = message.reactors,
            chat = chat,
            appManager = appManager,
            onDismiss = { isReactorsSheetOpen = false },
        )
    }
    if (isInfoOpen) {
        MessageInfoDialog(
            message = message,
            chat = chat,
            appManager = appManager,
            onDismiss = { isInfoOpen = false },
        )
    }
    if (isActionsSheetOpen) {
        MessageActionsSheet(
            message = message,
            parsedBody = parsed.body,
            reactions = reactions,
            onDismiss = { isActionsSheetOpen = false },
            onReact = { emoji ->
                isActionsSheetOpen = false
                pickReaction(emoji)
            },
            postReactionSuggestions = postReactionSuggestions,
            onShowFullReactionPicker = {
                isActionsSheetOpen = false
                isReactionPickerOpen = true
            },
            onReply = {
                isActionsSheetOpen = false
                onReply()
            },
            onCopy = {
                isActionsSheetOpen = false
                clipboard.setText("Message", copyableMessageText(message))
            },
            onInfo = {
                isActionsSheetOpen = false
                isInfoOpen = true
            },
            onDelete = {
                isActionsSheetOpen = false
                onDelete()
            },
        )
    }
    if (isReactionPickerOpen) {
        IrisEmojiPickerSheet(
            onDismiss = { isReactionPickerOpen = false },
            suggestedEmojis = postReactionSuggestions,
            onPick = { emoji ->
                isReactionPickerOpen = false
                pickReaction(emoji)
            },
        )
    }
    val bubbleShape =
        messageBubbleShape(
            isOutgoing = message.isOutgoing,
            isFirstInCluster = isFirstInCluster,
            isLastInCluster = isLastInCluster,
        )
    val swipeOffsetX = remember(message.id) { Animatable(0f) }
    val density = LocalDensity.current
    val swipeThresholdPx = with(density) { 60.dp.toPx() }
    val swipeMaxOffsetPx = with(density) { 90.dp.toPx() }
    val swipeScope = rememberCoroutineScope()
    var swipeFedHaptic by remember(message.id) { mutableStateOf(false) }
    val swipeRevealForward =
        ((swipeOffsetX.value / swipeThresholdPx).coerceIn(0f, 1f))
    val swipeRevealBackward =
        ((-swipeOffsetX.value / swipeThresholdPx).coerceIn(0f, 1f))

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .hoverable(hoverInteractionSource),
    ) {
        Row(
            modifier =
                Modifier
                    .align(Alignment.Center)
                    .fillMaxWidth()
                    .padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.AutoMirrored.Rounded.Reply,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier =
                    Modifier
                        .size(20.dp)
                        .alpha(swipeRevealForward),
            )
            Spacer(modifier = Modifier.weight(1f))
            Icon(
                imageVector = Icons.Rounded.Info,
                contentDescription = null,
                tint = IrisTheme.palette.muted,
                modifier =
                    Modifier
                        .size(20.dp)
                        .alpha(swipeRevealBackward),
            )
        }
        Column(
            horizontalAlignment = if (message.isOutgoing) Alignment.End else Alignment.Start,
            verticalArrangement = Arrangement.spacedBy(4.dp),
            modifier =
                Modifier
                    .align(if (message.isOutgoing) Alignment.CenterEnd else Alignment.CenterStart)
                    .offset { IntOffset(swipeOffsetX.value.toInt(), 0) }
                    .pointerInput(message.id) {
                        detectHorizontalDragGestures(
                            onDragStart = { swipeFedHaptic = false },
                            onDragEnd = {
                                val finalOffset = swipeOffsetX.value
                                if (finalOffset >= swipeThresholdPx) {
                                    onReply()
                                } else if (finalOffset <= -swipeThresholdPx) {
                                    isInfoOpen = true
                                }
                                swipeScope.launch {
                                    swipeOffsetX.animateTo(
                                        targetValue = 0f,
                                        animationSpec = spring(
                                            stiffness = 350f,
                                            dampingRatio = 0.74f,
                                        ),
                                    )
                                }
                                swipeFedHaptic = false
                            },
                            onDragCancel = {
                                swipeScope.launch { swipeOffsetX.animateTo(0f) }
                                swipeFedHaptic = false
                            },
                            onHorizontalDrag = { change, dragAmount ->
                                change.consume()
                                val target =
                                    (swipeOffsetX.value + dragAmount).coerceIn(
                                        -swipeMaxOffsetPx,
                                        swipeMaxOffsetPx,
                                    )
                                swipeScope.launch { swipeOffsetX.snapTo(target) }
                                val crossed = abs(target) >= swipeThresholdPx
                                if (crossed && !swipeFedHaptic) {
                                    hapticFeedback.performHapticFeedback(
                                        HapticFeedbackType.LongPress,
                                    )
                                    swipeFedHaptic = true
                                } else if (!crossed) {
                                    swipeFedHaptic = false
                                }
                            },
                        )
                    },
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (showActionDock && message.isOutgoing) {
                    MessageActionDock(
                        postReactionSuggestions = postReactionSuggestions,
                        onReact = { emoji -> pickReaction(emoji) },
                        onReply = onReply,
                        onInfo = { isInfoOpen = true },
                        onDelete = onDelete,
                    )
                }
                Surface(
                    modifier =
                        Modifier
                            .widthIn(max = 300.dp)
                            .clip(bubbleShape)
                            .combinedClickable(
                                hapticFeedbackEnabled = false,
                                onClick = {},
                                onLongClick = {
                                    if (!showDesktopActionDock) {
                                        hapticFeedback.performHapticFeedback(HapticFeedbackType.LongPress)
                                        isActionsSheetOpen = true
                                    }
                                },
                            )
                            .testTag("chatMessage-${message.id}"),
                    color =
                        if (message.isOutgoing) {
                            IrisTheme.palette.bubbleMine
                        } else {
                            IrisTheme.palette.bubbleTheirs
                        },
                    shape = bubbleShape,
                    tonalElevation = 0.dp,
                    shadowElevation = 0.dp,
                ) {
                    Column(
                        modifier =
                            Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
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
                            ReplyPreview(reply = reply, isOutgoing = message.isOutgoing, onTap = onScrollToQuote)
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
                                modifier = Modifier.align(Alignment.End),
                                horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.End),
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
                                    DeliveryGlyph(
                                        message.delivery,
                                        isOutgoing = true,
                                    )
                                }
                            }
                        }
                    }
                }
                if (showActionDock && !message.isOutgoing) {
                    MessageActionDock(
                        postReactionSuggestions = postReactionSuggestions,
                        onReact = { emoji -> pickReaction(emoji) },
                        onReply = onReply,
                        onInfo = { isInfoOpen = true },
                        onDelete = onDelete,
                    )
                }
            }
            if (reactions.isNotEmpty()) {
                ReactionRow(
                    reactions = reactions,
                    onTap = { isReactorsSheetOpen = true },
                    modifier =
                        Modifier
                            // Tuck the reaction pills up under the bubble's
                            // bottom edge — visually attached to the message
                            // rather than a separate row below it. Custom
                            // layout shifts the row up AND reports a smaller
                            // height so the next message follows naturally.
                            .layout { measurable, constraints ->
                                val placeable = measurable.measure(constraints)
                                val overlap = 14.dp.roundToPx()
                                layout(placeable.width, (placeable.height - overlap).coerceAtLeast(0)) {
                                    placeable.place(0, -overlap)
                                }
                            }
                            .padding(if (message.isOutgoing) PaddingValues(end = 6.dp) else PaddingValues(start = 6.dp)),
                )
            }
        }
    }
}

@Composable
private fun SystemMessageChip(message: ChatMessageSnapshot) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp),
        horizontalArrangement = Arrangement.Center,
    ) {
        Surface(
            color = IrisTheme.palette.panel.copy(alpha = 0.68f),
            shape = RoundedCornerShape(100.dp),
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp),
                horizontalArrangement = Arrangement.spacedBy(7.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    imageVector = Icons.Rounded.Info,
                    contentDescription = null,
                    tint = IrisTheme.palette.muted,
                    modifier = Modifier.size(14.dp),
                )
                Text(
                    text = message.body,
                    style = MaterialTheme.typography.labelMedium,
                    color = IrisTheme.palette.muted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun MessageActionDock(
    postReactionSuggestions: List<String>,
    onReact: (String) -> Unit,
    onReply: () -> Unit,
    onInfo: () -> Unit,
    onDelete: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    var reactionPickerOpen by remember { mutableStateOf(false) }
    Surface(
        color = IrisTheme.palette.toolbar,
        shape = RoundedCornerShape(100.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 4.dp, vertical = 3.dp),
            horizontalArrangement = Arrangement.spacedBy(1.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box {
                ActionDockIconButton(
                    icon = Icons.Rounded.AddReaction,
                    label = "React",
                    testTag = "messageReactButton",
                    onClick = { reactionPickerOpen = true },
                )
                ReactionPickerMenu(
                    expanded = reactionPickerOpen,
                    onDismiss = { reactionPickerOpen = false },
                    postReactionSuggestions = postReactionSuggestions,
                    onEmoji = { emoji ->
                        reactionPickerOpen = false
                        onReact(emoji)
                    },
                )
            }
            ActionDockIconButton(Icons.AutoMirrored.Rounded.Reply, "Reply", onClick = onReply)
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
private fun ReactionPickerMenu(
    expanded: Boolean,
    onDismiss: () -> Unit,
    postReactionSuggestions: List<String>,
    onEmoji: (String) -> Unit,
) {
    if (!expanded) return
    IrisEmojiPickerSheet(
        onDismiss = onDismiss,
        suggestedEmojis = postReactionSuggestions,
        onPick = onEmoji,
    )
}

internal fun postReactionSuggestionEmojis(reactions: List<MessageReactionSnapshot>): List<String> =
    reactions
        .filter { reaction ->
            val count = reaction.count.toString().toLongOrNull() ?: 0L
            count > if (reaction.reactedByMe) 1L else 0L
        }
        .map { it.emoji }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MessageActionsSheet(
    message: ChatMessageSnapshot,
    parsedBody: String,
    reactions: List<MessageReactionSnapshot>,
    onDismiss: () -> Unit,
    onReact: (String) -> Unit,
    postReactionSuggestions: List<String>,
    onShowFullReactionPicker: () -> Unit,
    onReply: () -> Unit,
    onCopy: () -> Unit,
    onInfo: () -> Unit,
    onDelete: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 8.dp)
                    .testTag("messageActionsSheet"),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            QuickReactionRow(
                postReactionSuggestions = postReactionSuggestions,
                onPick = onReact,
                onMore = onShowFullReactionPicker,
            )
            MessagePreviewCard(message = message, parsedBody = parsedBody, reactions = reactions)
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                MessageActionRow(
                    icon = Icons.AutoMirrored.Rounded.Reply,
                    label = "Reply",
                    onClick = onReply,
                )
                MessageActionRow(
                    icon = Icons.Rounded.ContentCopy,
                    label = "Copy",
                    onClick = onCopy,
                )
                MessageActionRow(
                    icon = Icons.Rounded.Info,
                    label = "Message info",
                    onClick = onInfo,
                )
                MessageActionRow(
                    icon = Icons.Rounded.Delete,
                    label = "Delete message",
                    destructive = true,
                    onClick = onDelete,
                )
            }
        }
    }
}

@Composable
private fun QuickReactionRow(
    postReactionSuggestions: List<String>,
    onPick: (String) -> Unit,
    onMore: () -> Unit,
) {
    val context = LocalContext.current.applicationContext
    val recentEmojis = remember { loadRecentReactionEmojis(context) }
    val emojis = remember(postReactionSuggestions, recentEmojis) {
        irisReactionQuickChoices(postReactionSuggestions, recentEmojis)
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(100.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            emojis.forEach { emoji ->
                QuickReactionButton(emoji = emoji, onClick = { onPick(emoji) })
            }
            Box(
                modifier =
                    Modifier
                        .size(38.dp)
                        .clip(CircleShape)
                        .clickable(onClick = onMore)
                        .testTag("messageReactButton"),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Rounded.AddReaction,
                    contentDescription = "More reactions",
                    tint = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
    }
}

@Composable
private fun QuickReactionButton(emoji: String, onClick: () -> Unit) {
    Box(
        modifier =
            Modifier
                .size(38.dp)
                .clip(CircleShape)
                .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Text(text = emoji, style = MaterialTheme.typography.titleLarge)
    }
}

@Composable
private fun MessagePreviewCard(
    message: ChatMessageSnapshot,
    parsedBody: String,
    reactions: List<MessageReactionSnapshot>,
) {
    val previewText =
        when {
            parsedBody.isNotBlank() -> parsedBody
            message.attachments.isNotEmpty() -> message.attachments.first().filename.ifBlank { "Attachment" }
            else -> ""
        }
    if (previewText.isBlank() && message.attachments.isEmpty() && reactions.isEmpty()) return
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = IrisTheme.palette.panel,
        shape = RoundedCornerShape(14.dp),
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                text = message.author,
                style = MaterialTheme.typography.labelMedium,
                color = IrisTheme.palette.muted,
                fontWeight = FontWeight.SemiBold,
            )
            if (previewText.isNotBlank()) {
                Text(
                    text = previewText,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (message.attachments.isNotEmpty() && previewText != message.attachments.first().filename) {
                Text(
                    text = "${message.attachments.size} attachment${if (message.attachments.size == 1) "" else "s"}",
                    style = MaterialTheme.typography.labelSmall,
                    color = IrisTheme.palette.muted,
                )
            }
            if (reactions.isNotEmpty()) {
                ReactionRow(reactions = reactions)
            }
        }
    }
}

@Composable
private fun MessageActionRow(
    icon: ImageVector,
    label: String,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    val tint =
        if (destructive) {
            MaterialTheme.colorScheme.error
        } else {
            MaterialTheme.colorScheme.onSurface
        }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .clickable(onClick = onClick)
                .padding(horizontal = 12.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = tint,
            modifier = Modifier.size(20.dp),
        )
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color = tint,
        )
    }
}

@Composable
private fun ActionDockIconButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    testTag: String? = null,
) {
    Box(
        modifier =
            Modifier
                .size(28.dp)
                .clip(CircleShape)
                .clickable(onClick = onClick)
                .then(if (testTag != null) Modifier.testTag(testTag) else Modifier),
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
    onTap: () -> Unit,
) {
    val collapsedLineLimit = 4
    Surface(
        color =
            if (isOutgoing) {
                MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.12f)
            } else {
                MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
            },
        shape = RoundedCornerShape(10.dp),
        // Stretch across the bubble Column's resolved width — when the
        // body Text below is wider, the reply preview matches it instead
        // of sitting as a narrow pill on the leading side.
        modifier = Modifier.fillMaxWidth().clickable { onTap() },
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                modifier =
                    Modifier
                        .width(3.dp)
                        .heightIn(min = 34.dp)
                        .clip(CircleShape)
                        .background(
                            (
                                if (isOutgoing) MaterialTheme.colorScheme.onPrimary
                                else MaterialTheme.colorScheme.onSurface
                            ).copy(alpha = 0.6f),
                        ),
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
                    maxLines = collapsedLineLimit,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MessageReactorsSheet(
    reactors: List<MessageReactor>,
    chat: CurrentChatSnapshot?,
    appManager: AppManager?,
    onDismiss: () -> Unit,
) {
    val account = appManager?.account?.collectAsStateWithLifecycle()?.value
    val visible = remember(reactors) { reactors.filter { it.emoji.isNotBlank() } }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(),
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 18.dp, vertical = 8.dp)
                    .testTag("messageReactorsSheet"),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = "Reactions",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurface,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 6.dp),
            )
            visible.forEach { reactor ->
                MessageInfoReactorRow(
                    info = participantInfo(reactor.author, chat = chat, account = account),
                    emoji = reactor.emoji,
                )
            }
        }
    }
}

@Composable
private fun ReactionRow(
    reactions: List<MessageReactionSnapshot>,
    modifier: Modifier = Modifier,
    onTap: (() -> Unit)? = null,
) {
    Row(
        modifier = modifier.then(if (onTap != null) Modifier.clickable { onTap() } else Modifier),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
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
internal fun TypingIndicatorBubble(
    names: List<String>,
    modifier: Modifier = Modifier,
) {
    val label =
        when {
            names.isEmpty() -> ""
            names.size == 1 -> "${names.first()} is typing"
            else -> "${names.first()} and ${names.size - 1} more are typing"
        }
    Surface(
        modifier =
            modifier
                .widthIn(max = 280.dp)
                .testTag("chatTypingIndicator"),
        color = IrisTheme.palette.panel.copy(alpha = 0.82f),
        shape = RoundedCornerShape(100.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 13.dp, vertical = 8.dp),
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

internal data class ReplyPreviewData(
    val author: String,
    val body: String,
)

internal data class ParsedReplyMessage(
    val reply: ReplyPreviewData?,
    val body: String,
)

internal fun replyEncodedMessage(
    reply: ChatMessageSnapshot?,
    text: String,
): String {
    if (reply == null) {
        return text
    }
    return "$ReplyMessagePrefix${reply.author}: ${replySnippet(reply)}\n\n$text"
}

internal fun parseReplyEncodedMessage(text: String): ParsedReplyMessage {
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

internal fun replySnippet(message: ChatMessageSnapshot): String {
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

@Composable
private fun MessageInfoDialog(
    message: ChatMessageSnapshot,
    chat: CurrentChatSnapshot?,
    appManager: AppManager?,
    onDismiss: () -> Unit,
) {
    val clipboard = rememberIrisClipboard()
    val palette = IrisTheme.palette
    val trace = message.deliveryTrace
    val account = appManager?.account?.collectAsStateWithLifecycle()?.value
    val resolveParticipant: (String) -> ParticipantInfo = { pubkeyHex ->
        participantInfo(pubkeyHex, chat = chat, account = account)
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
        dismissButton = {
            TextButton(
                onClick = {
                    clipboard.setText("Message info", messageInfoText(message, chat))
                },
            ) { Text("Copy info") }
        },
        title = { Text("Message info") },
        text = {
            Column(
                modifier =
                    Modifier
                        .heightIn(max = 520.dp)
                        .verticalScroll(rememberScrollState())
                        .testTag("messageInfoDialog"),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    DeliveryGlyph(message.delivery, isOutgoing = message.isOutgoing)
                    Text(
                        text = deliveryLabel(message.delivery),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }

                MessageInfoSection(title = "Status") {
                    MessageInfoValueRow("Time", messageInfoDateTime(message.createdAtSecs.toLong()))
                    message.expiresAtSecs?.let {
                        MessageInfoValueRow("Deletes", messageInfoDateTime(it.toLong()))
                    }
                    MessageInfoValueRow("Type", messageInfoKind(message))
                }

                MessageInfoSection(title = "People") {
                    if (message.isOutgoing) {
                        if (message.recipientDeliveries.isEmpty()) {
                            MessageInfoValueRow("Recipients", "No receipts")
                        } else {
                            message.recipientDeliveries.forEach { recipient ->
                                MessageInfoRecipientRow(
                                    info = resolveParticipant(recipient.ownerPubkeyHex),
                                    subtitle = messageInfoDateTime(recipient.updatedAtSecs.toLong()),
                                    delivery = recipient.delivery,
                                )
                            }
                        }
                    } else {
                        MessageInfoRecipientRow(
                            info = ParticipantInfo(
                                name = message.author,
                                pictureUrl = chat?.pictureUrl,
                                isMe = false,
                            ),
                            subtitle = messageInfoDateTime(message.createdAtSecs.toLong()),
                            delivery = message.delivery,
                        )
                    }
                }

                val channels = trace.transportChannels.map(::prettyTransportChannel)
                val queuedDeviceNpubs = trace.queuedProtocolTargets.map(::shortNpub)
                val hasTransport =
                    channels.isNotEmpty() ||
                        queuedDeviceNpubs.isNotEmpty() ||
                        !trace.lastTransportError.isNullOrBlank()
                if (hasTransport) {
                    MessageInfoSection(title = "Transport") {
                        if (channels.isNotEmpty()) {
                            MessageInfoMultiValueRow(
                                label = if (message.isOutgoing) "Sent over" else "Received over",
                                values = channels,
                            )
                        }
                        if (queuedDeviceNpubs.isNotEmpty()) {
                            MessageInfoMultiValueRow(
                                label = "Queued devices",
                                values = queuedDeviceNpubs,
                                monospaced = true,
                            )
                        }
                        trace.lastTransportError?.takeIf { it.isNotBlank() }?.let { error ->
                            MessageInfoValueRow("Last error", error)
                        }
                    }
                }

                MessageInfoSection(title = "IDs") {
                    MessageInfoValueRow(
                        label = "Message",
                        value = message.id,
                        monospaced = true,
                        copyValue = message.id,
                    )
                    message.sourceEventId?.takeIf { it.isNotBlank() }?.let { sourceEventId ->
                        MessageInfoValueRow(
                            label = "Received event",
                            value = shortMessageIdentifier(sourceEventId),
                            monospaced = true,
                            copyValue = sourceEventId,
                        )
                    }
                    if (trace.outerEventIds.isNotEmpty()) {
                        MessageInfoCopyList("Network events", trace.outerEventIds)
                    }
                    if (trace.targetDeviceIds.isNotEmpty()) {
                        MessageInfoCopyList(
                            label = "Target devices",
                            values = trace.targetDeviceIds.map { peerInputToNpub(it) },
                        )
                    }
                }

                if (message.attachments.isNotEmpty()) {
                    MessageInfoSection(title = "Attachments") {
                        message.attachments.forEach { attachment ->
                            MessageInfoValueRow(
                                label = if (attachment.filename.isBlank()) "File" else attachment.filename,
                                value = attachment.htreeUrl,
                                monospaced = true,
                                copyValue = attachment.htreeUrl,
                            )
                        }
                    }
                }

                if (message.reactions.isNotEmpty() || message.reactors.isNotEmpty()) {
                    MessageInfoSection(title = "Reactions") {
                        message.reactions.forEach { reaction ->
                            MessageInfoValueRow(reaction.emoji, "${reaction.count}")
                        }
                        message.reactors.forEach { reactor ->
                            MessageInfoReactorRow(
                                info = resolveParticipant(reactor.author),
                                emoji = reactor.emoji,
                            )
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun MessageInfoSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    IrisSectionCard(contentPadding = PaddingValues(14.dp)) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Column(verticalArrangement = Arrangement.spacedBy(2.dp), content = content)
    }
}

@Composable
private fun MessageInfoValueRow(
    label: String,
    value: String,
    monospaced: Boolean = false,
    copyValue: String? = null,
) {
    val palette = IrisTheme.palette
    val clipboard = rememberIrisClipboard()
    Row(
        modifier = Modifier.padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = label,
            modifier = Modifier.widthIn(min = 92.dp, max = 120.dp),
            style = MaterialTheme.typography.labelMedium,
            color = palette.muted,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text = value,
            modifier = Modifier.weight(1f),
            style =
                if (monospaced) {
                    MaterialTheme.typography.labelMedium.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)
                } else {
                    MaterialTheme.typography.bodyMedium
                },
            color = MaterialTheme.colorScheme.onSurface,
        )
        if (copyValue != null) {
            TextButton(onClick = { clipboard.setText(label, copyValue) }) { Text("Copy") }
        }
    }
}

@Composable
private fun MessageInfoMultiValueRow(
    label: String,
    values: List<String>,
    monospaced: Boolean = false,
) {
    val palette = IrisTheme.palette
    Column(
        modifier = Modifier.padding(vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = palette.muted,
            fontWeight = FontWeight.SemiBold,
        )
        values.forEach { value ->
            Text(
                text = value,
                style =
                    if (monospaced) {
                        MaterialTheme.typography.labelMedium.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)
                    } else {
                        MaterialTheme.typography.bodyMedium
                    },
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun MessageInfoCopyList(label: String, values: List<String>) {
    values.forEachIndexed { index, value ->
        MessageInfoValueRow(
            label = if (index == 0) label else "",
            value = shortMessageIdentifier(value),
            monospaced = true,
            copyValue = value,
        )
    }
}

@Composable
private fun MessageInfoRecipientRow(
    info: ParticipantInfo,
    subtitle: String,
    delivery: DeliveryState,
) {
    val palette = IrisTheme.palette
    Row(
        modifier = Modifier.padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(label = info.name, size = 32.dp, imageUrl = info.pictureUrl)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = info.name,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${deliveryLabel(delivery)} · $subtitle",
                style = MaterialTheme.typography.labelSmall,
                color = palette.muted,
            )
        }
        DeliveryGlyph(delivery, isOutgoing = true)
    }
}

private fun messageInfoText(message: ChatMessageSnapshot, chat: CurrentChatSnapshot? = null): String {
    val trace = message.deliveryTrace
    val lines =
        mutableListOf(
            "Message ${message.id}",
            "Time ${messageInfoDateTime(message.createdAtSecs.toLong())}",
            "Type ${messageInfoKind(message)}",
            "Status ${deliveryLabel(message.delivery)}",
        )
    message.expiresAtSecs?.let {
        lines += "Deletes ${messageInfoDateTime(it.toLong())}"
    }
    val channels = trace.transportChannels.map(::prettyTransportChannel)
    if (channels.isNotEmpty()) {
        lines += "${if (message.isOutgoing) "Sent over" else "Received over"} ${channels.joinToString(", ")}"
    }
    if (message.recipientDeliveries.isNotEmpty()) {
        lines += "Recipients"
        lines +=
            message.recipientDeliveries.map { recipient ->
                "- ${messageInfoRecipientName(recipient.ownerPubkeyHex, chat)} ${deliveryLabel(recipient.delivery)} ${messageInfoDateTime(recipient.updatedAtSecs.toLong())}"
            }
    } else if (!message.isOutgoing) {
        lines += "From ${message.author}"
        lines += "You ${deliveryLabel(message.delivery)}"
    }
    if (trace.outerEventIds.isNotEmpty()) {
        lines += "Network IDs ${shortMessageIdentifierList(trace.outerEventIds)}"
    }
    if (trace.queuedProtocolTargets.isNotEmpty()) {
        lines += "Queued devices ${trace.queuedProtocolTargets.joinToString(", ", transform = ::shortNpub)}"
    }
    if (trace.targetDeviceIds.isNotEmpty()) {
        lines += "Devices ${trace.targetDeviceIds.joinToString(", ", transform = ::shortNpub)}"
    }
    trace.lastTransportError?.takeIf { it.isNotBlank() }?.let { error ->
        lines += "Last send error $error"
    }
    message.sourceEventId?.takeIf { it.isNotBlank() }?.let { sourceEventId ->
        lines += "Received as ${shortMessageIdentifier(sourceEventId)}"
    }
    if (message.attachments.isNotEmpty()) {
        lines += "Attachments"
        lines +=
            message.attachments.map { attachment ->
                "- ${if (attachment.filename.isBlank()) "File" else attachment.filename} ${attachment.htreeUrl}"
            }
    }
    if (message.reactions.isNotEmpty()) {
        lines += "Reactions"
        lines += message.reactions.map { "- ${it.emoji} ${it.count}" }
    }
    return lines.joinToString("\n")
}

private fun messageInfoDirection(message: ChatMessageSnapshot): String =
    when {
        message.kind == ChatMessageKind.SYSTEM -> "System message"
        message.isOutgoing -> "Sent message"
        else -> "Received message"
    }

private fun messageInfoKind(message: ChatMessageSnapshot): String =
    when (message.kind) {
        ChatMessageKind.SYSTEM -> "System"
        ChatMessageKind.USER -> if (message.isOutgoing) "Sent" else "Received"
    }

private fun messageInfoRecipientName(ownerPubkeyHex: String, chat: CurrentChatSnapshot?): String {
    if (chat != null && chat.kind == ChatKind.DIRECT && chat.chatId == ownerPubkeyHex) {
        return chat.displayName
    }
    return shortNpub(ownerPubkeyHex)
}

private fun shortNpub(pubkeyInput: String): String {
    val npub = peerInputToNpub(pubkeyInput).ifBlank { pubkeyInput }
    return shortMessageIdentifier(npub)
}

private fun prettyTransportChannel(channel: String): String =
    when {
        channel.startsWith("message server: ") -> channel.removePrefix("message server: ")
        channel == "message servers" -> "Message server"
        else -> channel
    }

private data class ParticipantInfo(
    val name: String,
    val pictureUrl: String?,
    val isMe: Boolean,
)

private fun participantInfo(
    pubkeyHex: String,
    chat: CurrentChatSnapshot?,
    account: AccountSnapshot?,
): ParticipantInfo {
    if (account != null && account.publicKeyHex == pubkeyHex) {
        val name = account.displayName.trim().ifEmpty { "You" }
        return ParticipantInfo(name = name, pictureUrl = account.pictureUrl, isMe = true)
    }
    if (chat != null && chat.kind == ChatKind.DIRECT && chat.chatId == pubkeyHex) {
        return ParticipantInfo(name = chat.displayName, pictureUrl = chat.pictureUrl, isMe = false)
    }
    return ParticipantInfo(name = shortNpub(pubkeyHex), pictureUrl = null, isMe = false)
}

@Composable
private fun MessageInfoReactorRow(info: ParticipantInfo, emoji: String) {
    val palette = IrisTheme.palette
    Row(
        modifier = Modifier.padding(vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IrisAvatar(label = info.name, size = 32.dp, imageUrl = info.pictureUrl)
        Text(
            text = info.name,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (emoji.isBlank()) {
            Text(
                text = "Removed",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
        } else {
            Text(text = emoji, style = MaterialTheme.typography.titleLarge)
        }
    }
}

private fun messageInfoDateTime(secs: Long): String {
    val formatter = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
    return formatter.format(Date(secs * 1000L))
}

private fun shortMessageIdentifierList(values: List<String>): String =
    values.joinToString(", ") { shortMessageIdentifier(it) }

private fun shortMessageIdentifier(value: String): String =
    if (value.length <= 16) value else "${value.take(8)}...${value.takeLast(8)}"

private fun deliveryLabel(delivery: DeliveryState): String =
    when (delivery) {
        DeliveryState.QUEUED -> "Queued"
        DeliveryState.PENDING -> "Pending"
        DeliveryState.SENT -> "Sent"
        DeliveryState.RECEIVED -> "Received"
        DeliveryState.SEEN -> "Seen"
        DeliveryState.FAILED -> "Failed"
    }

private const val MessageClusterGapSecs = 60L
internal fun startsMessageCluster(
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

internal val ChatEmojiChoices =
    listOf(
        "😀", "😂", "😊", "😍", "🥰", "😎", "🤔", "😭",
        "❤️", "🔥", "✨", "🙏", "👍", "👀", "🎉", "💜",
        "🌞", "🌙", "⭐️", "🍓", "☕️", "🌊", "🚀", "✅",
    )
