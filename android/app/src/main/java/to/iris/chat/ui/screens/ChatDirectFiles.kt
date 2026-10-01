package to.iris.chat.ui.screens

import android.content.Context
import android.content.Intent
import android.text.format.Formatter
import android.webkit.MimeTypeMap
import android.widget.Toast
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.ButtonDefaults
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DirectFileSnapshot
import to.iris.chat.rust.DirectFileTransferSnapshot
import to.iris.chat.rust.DirectFileTransferStatus
import to.iris.chat.rust.OutgoingAttachment

// Adding another source must never turn a staged direct file into an upload.
internal fun directFileSendMode(current: Boolean, hasFiles: Boolean, selectedDirectly: Boolean): Boolean =
    selectedDirectly || (current && hasFiles)

internal fun attachmentSendAction(
    chatId: String,
    attachments: List<PickedAttachment>,
    caption: String,
    sendDirectly: Boolean,
): AppAction {
    val files = attachments.map { OutgoingAttachment(it.path, it.filename) }
    return if (sendDirectly) AppAction.SendDirectFiles(chatId, files, caption)
    else AppAction.SendAttachments(chatId, files, caption)
}

internal val DirectFileTransferSnapshot.canAcceptOnThisDevice: Boolean
    get() = status == DirectFileTransferStatus.OFFERED && !isSender

internal val DirectFileTransferSnapshot.canCancelOnThisDevice: Boolean
    get() = (status == DirectFileTransferStatus.OFFERED && isSender) ||
        status == DirectFileTransferStatus.CONNECTING || status == DirectFileTransferStatus.TRANSFERRING

internal val DirectFileTransferSnapshot.displayStatus: String
    get() = when (status) {
        DirectFileTransferStatus.OFFERED -> if (isSender) "Waiting for acceptance" else "Ready to receive"
        DirectFileTransferStatus.CONNECTING -> "Connecting…"
        DirectFileTransferStatus.TRANSFERRING -> if (isSender) "Sending…" else "Receiving…"
        DirectFileTransferStatus.COMPLETED -> if (isSender) "Sent" else "Received"
        DirectFileTransferStatus.DECLINED -> "Declined"
        DirectFileTransferStatus.CANCELLED -> "Cancelled"
        DirectFileTransferStatus.FAILED -> "Transfer failed"
        DirectFileTransferStatus.UNAVAILABLE -> "Files unavailable"
    }

@Composable
internal fun ChatDirectFileTransfer(
    transfer: DirectFileTransferSnapshot,
    chatId: String,
    isOutgoing: Boolean = false,
    dispatch: (AppAction) -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    fun open(file: DirectFileSnapshot, share: Boolean) {
        scope.launch {
            if (!openDirectFile(context, file, share)) {
                Toast.makeText(context, "Couldn’t open file", Toast.LENGTH_SHORT).show()
            }
        }
    }
    val foreground = if (isOutgoing) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
    val textButtons = ButtonDefaults.textButtonColors(contentColor = foreground)
    CompositionLocalProvider(LocalContentColor provides foreground) {
        Column(
            modifier = Modifier.widthIn(max = 260.dp).testTag("chatDirectTransfer-${transfer.id}"),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Direct files", style = MaterialTheme.typography.titleSmall)
            transfer.files.forEachIndexed { index, file ->
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(file.filename, style = MaterialTheme.typography.bodyMedium, maxLines = 2)
                    Text(Formatter.formatFileSize(context, file.sizeBytes.toLong()), style = MaterialTheme.typography.labelSmall)
                    if (transfer.status == DirectFileTransferStatus.COMPLETED && file.localPath != null) {
                        Row {
                            TextButton(onClick = { open(file, false) }, colors = textButtons, modifier = Modifier.testTag("chatDirectTransferOpen-${transfer.id}-$index")) { Text("Open") }
                            TextButton(onClick = { open(file, true) }, colors = textButtons) { Text("Share") }
                        }
                    }
                }
            }
            Text(transfer.displayStatus, style = MaterialTheme.typography.labelMedium)
            if (transfer.status == DirectFileTransferStatus.CONNECTING || transfer.status == DirectFileTransferStatus.TRANSFERRING) {
                LinearProgressIndicator(
                    progress = { if (transfer.totalBytes == 0uL) 0f else (transfer.transferredBytes.toDouble() / transfer.totalBytes.toDouble()).toFloat().coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth(),
                    color = foreground,
                    trackColor = foreground.copy(alpha = 0.2f),
                )
                Text(
                    "${Formatter.formatFileSize(context, transfer.transferredBytes.toLong())} of ${Formatter.formatFileSize(context, transfer.totalBytes.toLong())}",
                    style = MaterialTheme.typography.labelSmall,
                )
            }
            if (transfer.canAcceptOnThisDevice) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { dispatch(AppAction.AcceptDirectFiles(chatId, transfer.id)) }, modifier = Modifier.testTag("chatDirectTransferAccept-${transfer.id}")) { Text("Accept") }
                    TextButton(onClick = { dispatch(AppAction.DeclineDirectFiles(chatId, transfer.id)) }, colors = textButtons, modifier = Modifier.testTag("chatDirectTransferDecline-${transfer.id}")) { Text("Decline") }
                }
            } else if (transfer.canCancelOnThisDevice) {
                TextButton(onClick = { dispatch(AppAction.CancelDirectFiles(chatId, transfer.id)) }, colors = textButtons, modifier = Modifier.testTag("chatDirectTransferCancel-${transfer.id}")) { Text("Cancel") }
            }
        }
    }
}

// Sharing uses the existing, narrow attachment cache provider. Received files
// remain private until the user opens or shares an individual completed file.
private suspend fun openDirectFile(context: Context, file: DirectFileSnapshot, share: Boolean): Boolean =
    runCatching {
        val output = withContext(Dispatchers.IO) {
            val source = File(requireNotNull(file.localPath))
            check(source.isFile)
            val directory = File(context.cacheDir, "attachments/direct-${UUID.randomUUID()}").apply { mkdirs() }
            val name = File(file.filename.replace('\\', '/')).name.ifBlank { "file" }
            source.copyTo(File(directory, name))
        }
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", output)
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(output.extension.lowercase()) ?: "application/octet-stream"
        val intent = Intent(if (share) Intent.ACTION_SEND else Intent.ACTION_VIEW).apply {
            if (share) {
                type = mime
                putExtra(Intent.EXTRA_STREAM, uri)
            } else setDataAndType(uri, mime)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        context.startActivity(Intent.createChooser(intent, file.filename).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        true
    }.getOrDefault(false)
