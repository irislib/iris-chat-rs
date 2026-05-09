package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun ReplyComposerStrip(
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

@Composable
internal fun ComposerBar(
    draft: String,
    selectedAttachments: List<PickedAttachment>,
    isSending: Boolean,
    isUploading: Boolean,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
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
            modifier
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
                            (focusRequester?.let { Modifier.focusRequester(it) } ?: Modifier)
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
            ChatEmojiChoices.forEach { emoji ->
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
