package to.iris.chat.ui.screens

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.Toast
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.util.Locale
import to.iris.chat.BuildConfig
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.peerInputToNpub
import to.iris.chat.ui.components.IrisClipboard
import to.iris.chat.ui.theme.IrisTheme

private const val IrisSupportEmail = "irismessenger@pm.me"

fun isUserBlocked(
    preferences: PreferencesSnapshot,
    userId: String,
): Boolean {
    val normalized = userId.trim().lowercase(Locale.ROOT)
    return normalized.isNotEmpty() && preferences.blockedOwnerPubkeys.contains(normalized)
}

fun reportUser(
    context: android.content.Context,
    appManager: AppManager,
    clipboard: IrisClipboard,
    chatId: String,
    displayName: String,
    block: Boolean,
) {
    if (block) {
        appManager.dispatch(AppAction.SetUserBlocked(chatId, true))
    }

    val body =
        """
        Reported user: $displayName
        User ID: ${peerInputToNpub(chatId)}
        App: Iris Chat ${BuildConfig.VERSION_NAME}

        What happened:
        """.trimIndent()
    val intent =
        Intent(Intent.ACTION_SENDTO).apply {
            data = Uri.parse("mailto:")
            putExtra(Intent.EXTRA_EMAIL, arrayOf(IrisSupportEmail))
            putExtra(Intent.EXTRA_SUBJECT, "Iris Chat user report")
            putExtra(Intent.EXTRA_TEXT, body)
        }

    try {
        context.startActivity(intent)
    } catch (_: ActivityNotFoundException) {
        clipboard.setText(
            "User report",
            "To: $IrisSupportEmail\nSubject: Iris Chat user report\n\n$body",
        )
        Toast.makeText(context, "Report details copied", Toast.LENGTH_SHORT).show()
    }
}

@Composable
fun BlockedComposerBar(
    onUnblock: () -> Unit,
    onDelete: () -> Unit,
) {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("blockedComposerBar"),
        color = IrisTheme.palette.panelRaised,
        tonalElevation = 2.dp,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    imageVector = Icons.Rounded.Block,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(20.dp),
                )
                Text(
                    text = "User blocked",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.weight(1f),
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(
                    onClick = onDelete,
                    modifier =
                        Modifier
                            .weight(1f)
                            .testTag("blockedDeleteChatButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Delete chat")
                }
                Button(
                    onClick = onUnblock,
                    modifier =
                        Modifier
                            .weight(1f)
                            .testTag("blockedUnblockButton"),
                    colors =
                        ButtonDefaults.buttonColors(
                            containerColor = IrisTheme.palette.accent,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                        ),
                ) {
                    Text("Unblock")
                }
            }
        }
    }
}

@Composable
fun MessageRequestBar(
    displayName: String,
    onBlock: () -> Unit,
    onBlockAndReport: () -> Unit,
    onAccept: () -> Unit,
) {
    Surface(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("messageRequestBar"),
        color = IrisTheme.palette.panelRaised,
        tonalElevation = 2.dp,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = "Message request from $displayName",
                style = MaterialTheme.typography.bodySmall,
                color = IrisTheme.palette.muted,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(
                    onClick = onBlock,
                    modifier =
                        Modifier
                            .weight(1f)
                            .testTag("messageRequestBlockButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block")
                }
                TextButton(
                    onClick = onBlockAndReport,
                    modifier =
                        Modifier
                            .weight(1f)
                            .testTag("messageRequestBlockAndReportButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block and report")
                }
                Button(
                    onClick = onAccept,
                    modifier =
                        Modifier
                            .weight(1f)
                            .testTag("messageRequestAcceptButton"),
                    colors =
                        ButtonDefaults.buttonColors(
                            containerColor = IrisTheme.palette.accent,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                        ),
                ) {
                    Text("Accept")
                }
            }
        }
    }
}

@Composable
fun MessageRequestBlockDialog(
    displayName: String,
    onDismiss: () -> Unit,
    onBlock: () -> Unit,
    onReportAndBlock: () -> Unit,
    onDelete: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Block $displayName?") },
        text = { Text("They will not be able to message you.") },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                TextButton(
                    onClick = onBlock,
                    modifier = Modifier.testTag("messageRequestBlockConfirmKeep"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block")
                }
                TextButton(
                    onClick = onReportAndBlock,
                    modifier = Modifier.testTag("messageRequestBlockAndReportButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block and report")
                }
                TextButton(
                    onClick = onDelete,
                    modifier = Modifier.testTag("messageRequestBlockDeleteChatButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Delete chat")
                }
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier.testTag("messageRequestBlockCancelButton"),
                ) {
                    Text("Cancel")
                }
            }
        },
    )
}

@Composable
fun MessageRequestBlockAndReportDialog(
    displayName: String,
    onDismiss: () -> Unit,
    onBlockAndReport: () -> Unit,
    onDelete: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Block and report $displayName?") },
        text = { Text("This prepares a report for support and blocks this user.") },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                TextButton(
                    onClick = onBlockAndReport,
                    modifier = Modifier.testTag("messageRequestBlockAndReportConfirmButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block and report")
                }
                TextButton(
                    onClick = onDelete,
                    modifier = Modifier.testTag("messageRequestBlockAndReportDeleteChatButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Delete chat")
                }
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier.testTag("messageRequestBlockAndReportCancelButton"),
                ) {
                    Text("Cancel")
                }
            }
        },
    )
}

@Composable
fun MessageRequestReportDialog(
    displayName: String,
    onDismiss: () -> Unit,
    onReport: () -> Unit,
    onReportAndBlock: () -> Unit,
    onDelete: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Report $displayName?") },
        text = { Text("This prepares a report for support.") },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                TextButton(
                    onClick = onReport,
                    modifier = Modifier.testTag("messageRequestReportButtonConfirm"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Report")
                }
                TextButton(
                    onClick = onReportAndBlock,
                    modifier = Modifier.testTag("messageRequestReportAndBlockButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Block and report")
                }
                TextButton(
                    onClick = onDelete,
                    modifier = Modifier.testTag("messageRequestReportDeleteChatButton"),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) {
                    Text("Delete chat")
                }
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier.testTag("messageRequestReportCancelButton"),
                ) {
                    Text("Cancel")
                }
            }
        },
    )
}
