package to.iris.chat.calls

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.ui.components.IrisMenuRow

private val qualityChoices = mapOf("auto" to "Automatic", "high" to "High quality", "data" to "Use less data", "custom" to "Custom")

@Composable
fun CallQualitySetting(app: AppManager, preferences: PreferencesSnapshot) {
    var open by remember { mutableStateOf(false) }
    IrisMenuRow("Call quality", onClick = { open = true }, subtitle = qualityChoices[preferences.callQuality] ?: "Automatic",
        modifier = Modifier.testTag("callQualitySetting"))
    if (open) CallQualityDialog(preferences.callQuality, preferences.callMaxBitrateBps, app::dispatch) { open = false }
}

@Composable
internal fun CallQualityDialog(quality: String, customMaxBitrateBps: UInt, onAction: (AppAction) -> Unit, onDismiss: () -> Unit) {
    var selected by remember { mutableStateOf(quality) }
    var bitrate by remember { mutableFloatStateOf((customMaxBitrateBps.toFloat() / 1_000_000).coerceIn(0.1f, 10f)) }
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Call quality") },
        text = {
            Column {
                qualityChoices.forEach { (value, label) ->
                    Row(Modifier.fillMaxWidth().clickable { selected = value }, verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(selected == value, onClick = { selected = value })
                        Text(label)
                    }
                }
                if (selected == "custom") {
                    Text("Maximum: %.1f Mbps".format(bitrate))
                    Slider(value = bitrate, onValueChange = { bitrate = it }, valueRange = 0.1f..10f,
                        modifier = Modifier.testTag("callQualityMaximum"))
                }
            }
        },
        confirmButton = { TextButton(onClick = {
            onAction(AppAction.SetCallQuality(selected, (bitrate * 1_000_000).toUInt()))
            onDismiss()
        }) { Text("Save") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } })
}
