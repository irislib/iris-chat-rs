package to.iris.chat.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Reply
import androidx.compose.material.icons.rounded.AddReaction
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.ui.components.IrisEmojiPickerSheet
import to.iris.chat.ui.components.IrisIcons
import to.iris.chat.ui.components.rememberIrisHapticFeedback
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun MessageActionDock(
    canReplyAndReact: Boolean,
    postReactionSuggestions: List<String>,
    onReact: (String) -> Unit,
    onReply: () -> Unit,
    onForward: () -> Unit,
    onCopy: () -> Unit,
    onInfo: () -> Unit,
    onDelete: () -> Unit,
    canCopyAndForward: Boolean = true,
    onEdit: (() -> Unit)? = null,
    onEditHistory: (() -> Unit)? = null,
    onDeleteForEveryone: (() -> Unit)? = null,
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
            if (canReplyAndReact) Box {
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
            if (canReplyAndReact) ActionDockIconButton(Icons.AutoMirrored.Rounded.Reply, "Reply", onClick = onReply)
            if (canCopyAndForward) ActionDockIconButton(IrisIcons.Share, "Forward", onClick = onForward)
            Box {
                ActionDockIconButton(Icons.Rounded.MoreHoriz, "More", { menuOpen = true })
                DropdownMenu(
                    expanded = menuOpen,
                    onDismissRequest = { menuOpen = false },
                ) {
                    onEdit?.let { action ->
                        DropdownMenuItem(text = { Text("Edit") }, onClick = { menuOpen = false; action() })
                    }
                    onEditHistory?.let { action ->
                        DropdownMenuItem(text = { Text("Edit history") }, onClick = { menuOpen = false; action() })
                    }
                    if (canCopyAndForward) DropdownMenuItem(
                        text = { Text("Copy text") },
                        onClick = {
                            menuOpen = false
                            onCopy()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text("Info") },
                        onClick = {
                            menuOpen = false
                            onInfo()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text("Delete for me") },
                        onClick = {
                            menuOpen = false
                            onDelete()
                        },
                    )
                    onDeleteForEveryone?.let { action ->
                        DropdownMenuItem(text = { Text("Delete for everyone") }, onClick = { menuOpen = false; action() })
                    }
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

@Composable
private fun ActionDockIconButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    testTag: String? = null,
) {
    val haptics = rememberIrisHapticFeedback()
    val interactionSource = remember { MutableInteractionSource() }
    Box(
        modifier =
            Modifier
                .size(28.dp)
                .clip(CircleShape)
                .clickable(
                    interactionSource = interactionSource,
                    indication = ripple(radius = 14.dp),
                ) {
                    haptics.press()
                    onClick()
                }
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
