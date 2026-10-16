package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.GroupDetailsSnapshot
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSectionCard
import to.iris.chat.ui.components.rememberIrisClipboard
import java.text.DateFormat
import java.util.Date

@Composable
internal fun GroupInviteLinks(appManager: AppManager, details: GroupDetailsSnapshot, creating: Boolean) {
    var expanded by remember(details.groupId) { mutableStateOf(false) }
    var maxUses by remember(details.groupId) { mutableStateOf<UInt?>(null) }
    var days by remember(details.groupId) { mutableStateOf(30) }
    var usesMenu by remember { mutableStateOf(false) }
    var expiryMenu by remember { mutableStateOf(false) }
    var shownCode by remember { mutableStateOf<String?>(null) }
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    IrisSectionCard {
        TextButton(onClick = { expanded = !expanded }) {
            Text("Invite links", style = MaterialTheme.typography.titleMedium)
        }
        if (expanded) {
            Row {
                Column {
                    TextButton(onClick = { usesMenu = true }) { Text("Uses: ${maxUses?.toString() ?: "No limit"}") }
                    DropdownMenu(expanded = usesMenu, onDismissRequest = { usesMenu = false }) {
                        listOf(null, 1u, 10u, 100u).forEach { limit ->
                            DropdownMenuItem(text = { Text(limit?.toString() ?: "No limit") }, onClick = { maxUses = limit; usesMenu = false })
                        }
                    }
                }
                Column {
                    TextButton(onClick = { expiryMenu = true }) { Text(if (days == 0) "Expires: Never" else "Expires: In $days days") }
                    DropdownMenu(expanded = expiryMenu, onDismissRequest = { expiryMenu = false }) {
                        listOf(7, 30, 0).forEach { value ->
                            DropdownMenuItem(text = { Text(if (value == 0) "Never" else "In $value days") }, onClick = { days = value; expiryMenu = false })
                        }
                    }
                }
            }
            IrisPrimaryButton(
                text = if (creating) "Creating…" else "Create link", enabled = !creating,
                modifier = Modifier.fillMaxWidth().testTag("groupInviteCreateButton"),
                onClick = { appManager.dispatch(AppAction.CreateGroupInvite(details.groupId, maxUses, if (days == 0) null else (days * 86400).toULong())) },
            )
            details.inviteLinks.forEach { invite ->
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(invite.title, style = MaterialTheme.typography.titleMedium)
                    Text(invite.maxUses?.let { "${invite.useCount} of $it joined" } ?: "${invite.useCount} joined", style = MaterialTheme.typography.bodySmall)
                    invite.expiresAt?.let { Text("Expires ${DateFormat.getDateInstance().format(Date(it.toLong() * 1000))}", style = MaterialTheme.typography.bodySmall) }
                    if (shownCode == invite.id) {
                        val bitmap = remember(invite.url) { createQrBitmap(invite.url, size = 768) }
                        if (bitmap != null) IrisQrCodeImage(bitmap = bitmap, contentDescription = invite.title, size = 260.dp, tag = "groupInviteQrCode")
                    }
                    Row {
                        TextButton(onClick = { clipboard.setText("Invite", invite.url) }) { Text("Copy") }
                        if (canShareText(context)) TextButton(onClick = { shareText(context, invite.url, invite.title) }) { Text("Share") }
                        TextButton(onClick = { shownCode = if (shownCode == invite.id) null else invite.id }) { Text(if (shownCode == invite.id) "Hide code" else "Show code") }
                        TextButton(onClick = { appManager.dispatch(AppAction.RevokeGroupInvite(details.groupId, invite.id)) }, modifier = Modifier.testTag("groupInviteRevokeButton")) { Text("Revoke", color = MaterialTheme.colorScheme.error) }
                    }
                }
            }
        }
    }
}
