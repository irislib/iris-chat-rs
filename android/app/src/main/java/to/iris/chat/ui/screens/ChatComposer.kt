package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun ReplyComposerStrip(
    message: ChatMessageSnapshot,
    onCancel: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val cancelInteractionSource = remember { MutableInteractionSource() }
    val authorName = if (message.isOutgoing) "You" else message.author
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.background)
                .padding(horizontal = 14.dp)
                .padding(top = 8.dp, bottom = 2.dp)
                .testTag("chatReplyComposer"),
    ) {
        Surface(
            modifier = Modifier.fillMaxWidth(),
            color = IrisTheme.palette.panelAlt,
            shape = RoundedCornerShape(12.dp),
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.Top,
            ) {
                Box(
                    modifier =
                        Modifier
                            .padding(vertical = 8.dp)
                            .size(width = 4.dp, height = 38.dp)
                            .clip(CircleShape)
                            .background(IrisTheme.palette.muted.copy(alpha = 0.55f)),
                )
                Column(
                    modifier =
                        Modifier
                            .weight(1f)
                            .padding(vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Text(
                        text = authorName,
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = replySnippet(message),
                        style = MaterialTheme.typography.bodySmall,
                        color = IrisTheme.palette.muted,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                message.attachments.firstOrNull()?.let { attachment ->
                    ReplyComposerAttachmentBadge(
                        icon = replyAttachmentIcon(attachment),
                        label = replyAttachmentLabel(attachment),
                        modifier = Modifier.padding(vertical = 6.dp),
                    )
                }
                Box(
                    modifier =
                        Modifier
                            .padding(top = 4.dp)
                            .size(32.dp)
                            .clip(CircleShape)
                            .clickable(
                                interactionSource = cancelInteractionSource,
                                indication = null,
                            ) {
                                haptics.press()
                                onCancel()
                            }
                            .testTag("chatReplyCancelButton"),
                    contentAlignment = Alignment.Center,
                ) {
                    Box(
                        modifier =
                            Modifier
                                .size(24.dp)
                                .clip(CircleShape)
                                .background(IrisTheme.palette.toolbar),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            imageVector = IrisIcons.Close,
                            contentDescription = "Cancel reply",
                            tint = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.size(15.dp),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ReplyComposerAttachmentBadge(
    icon: ImageVector,
    label: String,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.size(46.dp),
        color = IrisTheme.palette.toolbar,
        shape = RoundedCornerShape(10.dp),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(
                imageVector = icon,
                contentDescription = label,
                tint = IrisTheme.palette.muted,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

private fun replyAttachmentIcon(attachment: MessageAttachmentSnapshot): ImageVector =
    when {
        attachment.isImage -> IrisIcons.Image
        attachment.isVideo -> IrisIcons.Movie
        attachment.isAudio -> IrisIcons.Audio
        else -> IrisIcons.File
    }

private fun replyAttachmentLabel(attachment: MessageAttachmentSnapshot): String =
    when {
        attachment.isImage -> "Image"
        attachment.isVideo -> "Video"
        attachment.isAudio -> "Audio"
        else -> "File"
    }

@Composable
internal fun ProtocolReadinessBar(message: String) {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .testTag("protocolReadinessComposerBar"),
        color = MaterialTheme.colorScheme.background,
        tonalElevation = 0.dp,
    ) {
        Surface(
            modifier =
                Modifier
                    .padding(horizontal = 14.dp, vertical = 10.dp)
                    .fillMaxWidth(),
            color = IrisTheme.palette.panelAlt,
            shape = RoundedCornerShape(12.dp),
        ) {
            Text(
                text = message,
                style = MaterialTheme.typography.bodySmall,
                color = IrisTheme.palette.muted,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            )
        }
    }
}

@Composable
internal fun ComposerBar(
    draft: String,
    selectedAttachments: List<PickedAttachment>,
    isSending: Boolean,
    isUploading: Boolean,
    uploadFraction: Float?,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
    onDraftChange: (String) -> Unit,
    onAttach: () -> Unit,
    onRemoveAttachment: (PickedAttachment) -> Unit,
    onSend: () -> Unit,
) {
    val haptics = rememberIrisHapticFeedback()
    val isBusy = isSending || isUploading
    val hasText = draft.isNotBlank()
    val hasAttachment = selectedAttachments.isNotEmpty()
    val hasSendContent = hasText || hasAttachment
    val canSend = hasSendContent && !isBusy
    val trailingIsSend = hasSendContent || isSending
    val showTrailingProgress = isSending || (isUploading && hasSendContent)
    val showInlineAttach = hasText && !hasAttachment && !isUploading
    val density = LocalDensity.current
    val windowWidth = with(density) { LocalWindowInfo.current.containerSize.width.toDp() }
    val showDesktopComposerTools = windowWidth >= 600.dp
    var showingEmojiPicker by remember { mutableStateOf(false) }
    val attachInteractionSource = remember { MutableInteractionSource() }
    val emojiInteractionSource = remember { MutableInteractionSource() }
    val sendInteractionSource = remember { MutableInteractionSource() }
    fun submitDraft() {
        if (canSend) {
            haptics.confirm()
            onSend()
        }
    }

    // Signal-style: composer outer matches the chat backdrop so it
    // visually merges with the timeline above instead of reading as
    // a separate dock. The input pill below is the single visible
    // surface.
    Surface(
        modifier =
            modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .imePadding(),
        color = MaterialTheme.colorScheme.background,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 8.dp),
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
                    if (uploadFraction != null) {
                        LinearProgressIndicator(
                            progress = { uploadFraction.coerceIn(0f, 1f) },
                            modifier = Modifier.fillMaxWidth(),
                            color = IrisTheme.palette.accent,
                            trackColor = IrisTheme.palette.muted.copy(alpha = 0.18f),
                        )
                    } else {
                        LinearProgressIndicator(
                            modifier = Modifier.fillMaxWidth(),
                            color = IrisTheme.palette.accent,
                            trackColor = IrisTheme.palette.muted.copy(alpha = 0.18f),
                        )
                    }
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
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.Bottom,
            ) {
                Surface(
                    modifier =
                        Modifier
                            .weight(1f)
                            .heightIn(min = 44.dp),
                    color = IrisTheme.palette.panelAlt,
                    shape = RoundedCornerShape(22.dp),
                    tonalElevation = 0.dp,
                    shadowElevation = 0.dp,
                ) {
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .heightIn(min = 44.dp),
                        verticalAlignment = Alignment.Bottom,
                    ) {
                        if (showDesktopComposerTools) {
                            Box(
                                modifier =
                                    Modifier
                                        .size(44.dp)
                                        .clip(CircleShape)
                                        .clickable(
                                            enabled = !isBusy,
                                            interactionSource = emojiInteractionSource,
                                            indication = null,
                                        ) {
                                            haptics.press()
                                            showingEmojiPicker = !showingEmojiPicker
                                        }
                                        .testTag("chatEmojiButton"),
                                contentAlignment = Alignment.Center,
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

                        BasicTextField(
                            value = draft,
                            onValueChange = onDraftChange,
                            modifier =
                                (focusRequester?.let { Modifier.focusRequester(it) } ?: Modifier)
                                    .weight(1f)
                                    .heightIn(min = 44.dp, max = 132.dp)
                                    .padding(
                                        start = if (showDesktopComposerTools) 0.dp else 16.dp,
                                        end = if (showInlineAttach) 0.dp else 12.dp,
                                    )
                                    .testTag("chatMessageInput"),
                            textStyle =
                                MaterialTheme.typography.bodyLarge.copy(
                                    color = MaterialTheme.colorScheme.onSurface,
                                ),
                            cursorBrush = SolidColor(IrisTheme.palette.accent),
                            keyboardOptions =
                                KeyboardOptions(
                                    capitalization = KeyboardCapitalization.Sentences,
                                ),
                            minLines = 1,
                            maxLines = 5,
                            decorationBox = { innerTextField ->
                                Box(
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .heightIn(min = 44.dp)
                                            .padding(vertical = 8.dp),
                                    contentAlignment = Alignment.CenterStart,
                                ) {
                                    if (draft.isEmpty()) {
                                        Text(
                                            text = "Message",
                                            style = MaterialTheme.typography.bodyLarge,
                                            color = IrisTheme.palette.muted,
                                        )
                                    }
                                    innerTextField()
                                }
                            },
                        )

                        if (showInlineAttach) {
                            Box(
                                modifier =
                                    Modifier
                                        .size(44.dp)
                                        .clip(CircleShape)
                                        .clickable(
                                            enabled = !isBusy,
                                            interactionSource = attachInteractionSource,
                                            indication = null,
                                        ) {
                                            haptics.press()
                                            onAttach()
                                        }
                                        .testTag("chatInlineAttachButton"),
                                contentAlignment = Alignment.Center,
                            ) {
                                Icon(
                                    imageVector = Icons.Rounded.Add,
                                    contentDescription = "Add",
                                    tint =
                                        if (isBusy) {
                                            IrisTheme.palette.muted.copy(alpha = 0.54f)
                                        } else {
                                            MaterialTheme.colorScheme.onSurface
                                        },
                                    modifier = Modifier.size(24.dp),
                                )
                            }
                        }
                    }
                }

                Box(
                    modifier =
                        Modifier
                            .size(40.dp)
                            .clip(CircleShape)
                            .background(IrisTheme.palette.accent)
                            .clickable(
                                enabled = if (trailingIsSend) canSend else !isBusy,
                                interactionSource = sendInteractionSource,
                                indication = null,
                            ) {
                                if (trailingIsSend) {
                                    submitDraft()
                                } else {
                                    haptics.press()
                                    onAttach()
                                }
                            }
                            .testTag(if (trailingIsSend) "chatSendButton" else "chatAttachButton"),
                    contentAlignment = Alignment.Center,
                ) {
                    if (showTrailingProgress) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onPrimary,
                        )
                    } else {
                        Icon(
                            imageVector = if (trailingIsSend) IrisIcons.Send else Icons.Rounded.Add,
                            contentDescription = if (trailingIsSend) "Send" else "Add",
                            tint = MaterialTheme.colorScheme.onPrimary,
                            modifier = Modifier.size(if (trailingIsSend) 22.dp else 24.dp),
                        )
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
    val haptics = rememberIrisHapticFeedback()
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
            ChatEmojiChoices.forEach { emoji ->
                val interactionSource = remember(emoji) { MutableInteractionSource() }
                Box(
                    modifier =
                        Modifier
                            .size(36.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(
                                enabled = enabled,
                                interactionSource = interactionSource,
                                indication = null,
                            ) {
                                haptics.press()
                                onEmoji(emoji)
                            },
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
