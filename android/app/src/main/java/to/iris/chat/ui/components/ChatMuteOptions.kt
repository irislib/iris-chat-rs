package to.iris.chat.ui.components

import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import to.iris.chat.rust.AppAction

internal val chatMuteDurations = listOf("1 hour" to 3600uL, "8 hours" to 28800uL, "1 day" to 86400uL, "1 week" to 604800uL)

@Composable
fun rememberChatMuteAction(chatId: String, muted: Boolean, dispatch: (AppAction) -> Unit): () -> Unit {
    var showing by remember(chatId) { mutableStateOf(false) }
    if (showing) {
        AlertDialog(
            onDismissRequest = { showing = false },
            title = { Text("Mute notifications") },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    if (muted) TextButton(onClick = {
                        showing = false
                        dispatch(AppAction.SetChatMuted(chatId, false))
                    }, modifier = Modifier.fillMaxWidth()) { Text("Unmute") }
                    chatMuteDurations.forEach { (label, seconds) ->
                        TextButton(onClick = {
                            showing = false
                            dispatch(AppAction.SetChatMuteUntil(chatId, (System.currentTimeMillis() / 1000).toULong() + seconds))
                        }, modifier = Modifier.fillMaxWidth().testTag("chatMute$seconds")) { Text(label) }
                    }
                    TextButton(onClick = {
                        showing = false
                        dispatch(AppAction.SetChatMuted(chatId, true))
                    }, modifier = Modifier.fillMaxWidth().testTag("chatMuteAlways")) { Text("Always") }
                }
            },
            confirmButton = { TextButton(onClick = { showing = false }) { Text("Cancel") } },
        )
    }
    return { showing = true }
}
