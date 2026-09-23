package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.CallMissed
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.CallHistorySnapshot
import to.iris.chat.ui.components.formatMessageClock
import to.iris.chat.ui.theme.IrisTheme

@Composable
internal fun CallHistoryRow(call: CallHistorySnapshot) {
    val missed = call.outcome == "missed"
    val tint = if (missed) MaterialTheme.colorScheme.error else IrisTheme.palette.muted
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 10.dp)
            .testTag("callHistory-${call.callId}").semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = when {
                missed -> Icons.AutoMirrored.Filled.CallMissed
                call.video -> Icons.Filled.Videocam
                else -> Icons.Filled.Call
            },
            contentDescription = null,
            tint = tint,
            modifier = Modifier.size(18.dp),
        )
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(callHistoryTitle(call), style = MaterialTheme.typography.labelLarge, color = tint)
            val time = formatMessageClock(call.startedAtSecs.toLong())
            val detail = if (call.outcome == "answered") "$time · ${callHistoryDuration(call.durationSecs)}" else time
            Text(detail, style = MaterialTheme.typography.labelSmall, color = IrisTheme.palette.muted)
        }
    }
}

internal fun callHistoryTitle(call: CallHistorySnapshot): String {
    val kind = if (call.video) "video call" else "voice call"
    val label = when (call.outcome) {
        "missed" -> "Missed"
        "declined" -> "Declined"
        "canceled" -> "Canceled"
        else -> if (call.direction == "outgoing") "Outgoing" else "Incoming"
    }
    return "$label $kind"
}

internal fun callHistoryDuration(seconds: ULong): String {
    val secs = (seconds % 60UL).toString().padStart(2, '0')
    val minutes = seconds / 60UL
    return if (minutes < 60UL) "$minutes:$secs"
        else "${minutes / 60UL}:${(minutes % 60UL).toString().padStart(2, '0')}:$secs"
}
