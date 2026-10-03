package to.iris.chat.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PauseCircle
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.DeviceHistorySyncPhase
import to.iris.chat.rust.DeviceHistorySyncSnapshot
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun DeviceHistorySyncProgress(progress: DeviceHistorySyncSnapshot) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.background)
            .navigationBarsPadding()
            .padding(horizontal = 20.dp, vertical = 12.dp)
            .testTag("deviceHistorySyncStatus"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (progress.phase == DeviceHistorySyncPhase.WAITING) {
            Icon(Icons.Default.PauseCircle, contentDescription = null, tint = IrisTheme.palette.muted)
        } else {
            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
        }
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                text = if (progress.phase == DeviceHistorySyncPhase.WAITING) "Waiting for your other device…" else "Syncing messages…",
                style = MaterialTheme.typography.bodyMedium,
            )
            progress.totalMessages?.let { total ->
                val numbers = java.text.NumberFormat.getIntegerInstance()
                Text(
                    text = "${numbers.format(progress.importedMessages.toLong())} of ${numbers.format(total.toLong())}",
                    modifier = Modifier.testTag("deviceHistorySyncCount"),
                    style = MaterialTheme.typography.bodySmall,
                    color = IrisTheme.palette.muted,
                )
            }
        }
    }
}
