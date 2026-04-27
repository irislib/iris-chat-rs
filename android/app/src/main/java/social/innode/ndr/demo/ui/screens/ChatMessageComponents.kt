package social.innode.ndr.demo.ui.screens

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Reply
import androidx.compose.material.icons.rounded.AddReaction
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import social.innode.ndr.demo.rust.ChatKind
import social.innode.ndr.demo.rust.ChatMessageKind
import social.innode.ndr.demo.rust.ChatMessageSnapshot
import social.innode.ndr.demo.rust.MessageAttachmentSnapshot
import social.innode.ndr.demo.rust.MessageReactionSnapshot
import social.innode.ndr.demo.ui.components.DeliveryGlyph
import social.innode.ndr.demo.ui.components.formatMessageClock
import social.innode.ndr.demo.ui.components.isSameTimelineDay
import social.innode.ndr.demo.ui.components.messageBubbleShape
import social.innode.ndr.demo.ui.components.rememberIrisClipboard
import social.innode.ndr.demo.ui.theme.IrisTheme

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
    downloadAttachment: suspend (MessageAttachmentSnapshot) -> ByteArray?,
    onOpenImage: (ByteArray, String) -> Unit,
) {
    if (message.kind == ChatMessageKind.SYSTEM) {
        SystemMessageChip(message = message)
        return
    }

    val clipboard = rememberIrisClipboard()
    val parsed = remember(message.body) { parseReplyEncodedMessage(message.body) }
    val showDesktopActionDock = LocalConfiguration.current.screenWidthDp >= 600
    val hoverInteractionSource = remember { MutableInteractionSource() }
    val isHovering by hoverInteractionSource.collectIsHoveredAsState()
    var isMobileActionDockOpen by remember(message.id) { mutableStateOf(false) }
    val showActionDock =
        (showDesktopActionDock && isHovering) || (!showDesktopActionDock && isMobileActionDockOpen)
    val bubbleShape =
        messageBubbleShape(
            isOutgoing = message.isOutgoing,
            isFirstInCluster = isFirstInCluster,
            isLastInCluster = isLastInCluster,
        )
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
                if (showActionDock && message.isOutgoing) {
                    MessageActionDock(
                        onReact = onReact,
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
                        Modifier
                            .clip(bubbleShape)
                            .combinedClickable(
                                onClick = {
                                    if (!showDesktopActionDock) {
                                        isMobileActionDockOpen = !isMobileActionDockOpen
                                    }
                                },
                                onLongClick = {
                                    clipboard.setText("Message", copyableMessageText(message))
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
                        onReact = onReact,
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
    onEmoji: (String) -> Unit,
) {
    DropdownMenu(
        expanded = expanded,
        onDismissRequest = onDismiss,
        modifier =
            Modifier
                .widthIn(max = 324.dp)
                .testTag("messageReactionPicker"),
    ) {
        Row(
            modifier =
                Modifier
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 8.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ChatEmojiChoices.forEachIndexed { index, emoji ->
                Box(
                    modifier =
                        Modifier
                            .size(36.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable { onEmoji(emoji) }
                            .testTag("messageReactionEmoji-$index"),
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

private data class ReplyPreviewData(
    val author: String,
    val body: String,
)

private data class ParsedReplyMessage(
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
