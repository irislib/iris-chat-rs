package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.FollowedUserSearchResult

@Composable
internal fun BlockedPeopleSettings(people: List<FollowedUserSearchResult>, dispatch: (AppAction) -> Unit) {
    Column(Modifier.fillMaxWidth().testTag("settingsBlockedPeople"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Blocked people", style = MaterialTheme.typography.titleMedium)
        if (people.isEmpty()) {
            Text("No blocked people", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        people.forEach { person ->
            Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(person.displayLabel, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(person.userId, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                TextButton(onClick = { dispatch(AppAction.SetUserBlocked(person.ownerPubkeyHex, false)) },
                    modifier = Modifier.testTag("settingsUnblock-${person.ownerPubkeyHex}")) {
                    Text("Unblock")
                }
            }
        }
    }
}
