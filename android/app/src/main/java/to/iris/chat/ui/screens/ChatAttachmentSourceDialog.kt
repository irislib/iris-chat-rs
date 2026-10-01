package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.SwapVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import to.iris.chat.ui.components.IrisIcons

@Composable
internal fun ChatAttachmentSourceDialog(onDismiss: () -> Unit, onPick: (Boolean) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add files") },
        text = { ChatAttachmentSourceRow(onPick = onPick) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
internal fun ChatAttachmentSourceRow(modifier: Modifier = Modifier, onPick: (Boolean) -> Unit) {
    Row(
        modifier = modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).testTag("chatAttachmentSources"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        AttachmentSource("Files", IrisIcons.File, "chatAttachmentFilesButton") { onPick(false) }
        AttachmentSource("Send directly", Icons.Rounded.SwapVert, "chatDirectFileButton") { onPick(true) }
    }
}

@Composable
private fun AttachmentSource(label: String, icon: ImageVector, tag: String, onClick: () -> Unit) {
    Column(
        modifier = Modifier.widthIn(min = 88.dp).clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = 4.dp).testTag(tag),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            modifier = Modifier.size(width = 76.dp, height = 54.dp)
                .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(20.dp)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(24.dp))
        }
        Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, softWrap = false)
    }
}
