package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import to.iris.chat.core.AppManager
import to.iris.chat.rust.ChatMessageSnapshot

@Composable
internal fun MessageEditHistoryDialog(
    message: ChatMessageSnapshot,
    appManager: AppManager?,
    onDismiss: () -> Unit,
) {
    val appState = appManager?.state?.collectAsStateWithLifecycle()
    val accountAtOpen = remember(appManager, message.chatId, message.id) {
        appState?.value?.account?.publicKeyHex
    }
    val candidate = if (appManager == null) message else {
        appState?.value?.takeIf {
            val active = it.router.screenStack.lastOrNull() ?: it.router.defaultScreen
            it.account?.publicKeyHex == accountAtOpen && it.account != null &&
                active is to.iris.chat.rust.Screen.Chat && active.chatId == message.chatId
        }
            ?.currentChat?.takeIf { it.chatId == message.chatId }
            ?.messages?.find { it.id == message.id }
    }
    val current = candidate?.takeIf {
        !it.deletedForEveryone && it.editHistory.isNotEmpty() &&
            (it.expiresAtSecs?.let { expiry -> expiry > (System.currentTimeMillis() / 1000).toULong() } != false)
    }
    LaunchedEffect(current == null) {
        if (current == null) onDismiss()
    }
    LaunchedEffect(current?.expiresAtSecs) {
        current?.expiresAtSecs?.let { expiry ->
            delay((expiry.toLong() * 1000 - System.currentTimeMillis()).coerceAtLeast(0))
            onDismiss()
        }
    }
    if (current == null) return
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Edit history") },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
        text = {
            SelectionContainer {
                Column(
                    modifier = Modifier.heightIn(max = 520.dp).verticalScroll(rememberScrollState()).testTag("messageEditHistory"),
                    verticalArrangement = Arrangement.spacedBy(18.dp),
                ) {
                    current.editHistory.asReversed().forEachIndexed { index, version ->
                        Column(
                            modifier = Modifier.testTag("messageEditVersion-${version.id}"),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                when (index) {
                                    0 -> "Current"
                                    current.editHistory.lastIndex -> "Original"
                                    else -> "Edit ${current.editHistory.lastIndex - index}"
                                },
                                style = MaterialTheme.typography.titleSmall,
                            )
                            Text(messageInfoDateTime(version.createdAtSecs.toLong()), style = MaterialTheme.typography.labelSmall)
                            Text(parseReplyEncodedMessage(version.body).body)
                        }
                    }
                }
            }
        },
    )
}
