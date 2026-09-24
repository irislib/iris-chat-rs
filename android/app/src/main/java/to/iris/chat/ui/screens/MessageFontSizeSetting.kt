package to.iris.chat.ui.screens

import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import to.iris.chat.core.AppManager
import to.iris.chat.ui.theme.MessageFontSize

@Composable
internal fun MessageFontSizeSetting(app: AppManager) {
    val selected by app.messageFontSize.size.collectAsStateWithLifecycle()
    MessageFontSizeSetting(selected) { app.messageFontSize.set(it) }
}

@Composable
internal fun MessageFontSizeSetting(selected: MessageFontSize, onSelect: (MessageFontSize) -> Unit) {
    var showingChoices by remember { mutableStateOf(false) }
    ListItem(
        headlineContent = { Text("Message font size") },
        supportingContent = { Text(selected.label) },
        modifier = Modifier.clickable { showingChoices = true }.testTag("messageFontSizeSetting"),
    )
    if (showingChoices) {
        AlertDialog(
            onDismissRequest = { showingChoices = false },
            title = { Text("Message font size") },
            text = {
                Column {
                    MessageFontSize.entries.forEach { size ->
                        Row(
                            modifier = Modifier.fillMaxWidth().selectable(selected = selected == size, role = Role.RadioButton, onClick = {
                                onSelect(size)
                                showingChoices = false
                            }).testTag("messageFontSize${size.name}").padding(vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = selected == size, onClick = null)
                            Text(size.label, fontSize = size.body.sp, modifier = Modifier.padding(start = 12.dp))
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { showingChoices = false }) { Text("Cancel") } },
        )
    }
}
