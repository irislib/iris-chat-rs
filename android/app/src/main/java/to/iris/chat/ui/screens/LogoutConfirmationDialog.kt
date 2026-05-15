package to.iris.chat.ui.screens

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import to.iris.chat.ui.components.rememberIrisHapticFeedback

@Composable
fun DeleteAppDataConfirmationDialog(
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
    confirmTag: String,
) {
    val haptics = rememberIrisHapticFeedback()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delete all local data?") },
        text = {
            Text("This removes your secret keys, messages, and cached files from this device.")
        },
        dismissButton = {
            TextButton(
                onClick = {
                    haptics.press()
                    onDismiss()
                },
                colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurface),
            ) {
                Text("Cancel")
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    haptics.confirm()
                    onConfirm()
                },
                modifier = Modifier.testTag(confirmTag),
                colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
            ) {
                Text("Delete")
            }
        },
    )
}
