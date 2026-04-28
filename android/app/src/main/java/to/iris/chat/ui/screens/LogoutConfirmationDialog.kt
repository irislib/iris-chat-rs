package to.iris.chat.ui.screens

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag

@Composable
fun DeleteAppDataConfirmationDialog(
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
    confirmTag: String,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delete app data?") },
        text = {
            Text("This removes your secret keys, messages, and cached files from this device.")
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
        confirmButton = {
            TextButton(
                onClick = onConfirm,
                modifier = Modifier.testTag(confirmTag),
            ) {
                Text(
                    text = "Delete",
                    color = MaterialTheme.colorScheme.error,
                )
            }
        },
    )
}
