package to.iris.chat.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.CurrentChatSnapshot

@Composable
fun IrisContactActions(appManager: AppManager, chat: CurrentChatSnapshot) {
    val contact = chat.contactIdentity ?: return
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            TextButton(
                onClick = { appManager.dispatch(AppAction.SetPublicFollow(chat.chatId, !contact.isFollowing)) },
                enabled = contact.canFollow && !contact.updatingFollow,
                modifier = Modifier.testTag("publicFollowButton"),
            ) { Text(if (contact.updatingFollow) "Saving…" else if (contact.isFollowing) "Unfollow (public)" else "Follow (public)") }
            TextButton(
                onClick = { appManager.dispatch(AppAction.SetContactFavorite(chat.chatId, !contact.isFavorite)) },
                modifier = Modifier.testTag("contactFavoriteButton"),
            ) {
                Icon(if (contact.isFavorite) Icons.Filled.Star else Icons.Outlined.StarBorder, contentDescription = null)
                Text(if (contact.isFavorite) "Favorited" else "Favorite")
            }
        }
        Text("Favorites are only visible to you", style = MaterialTheme.typography.bodySmall)
        IrisNameChangeNotice(appManager, chat)
        contact.firstSeenName?.takeIf { it != contact.savedName }?.let { name ->
            Text("First known as $name", style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
fun IrisNameChangeNotice(appManager: AppManager, chat: CurrentChatSnapshot) {
    val proposed = chat.contactIdentity?.pendingName ?: return
    Surface(modifier = Modifier.fillMaxWidth().testTag("contactNameChangeNotice"), tonalElevation = 2.dp) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text("New profile name: $proposed", style = MaterialTheme.typography.bodyMedium)
            TextButton(
                onClick = { appManager.dispatch(AppAction.ApproveContactName(chat.chatId, proposed)) },
                modifier = Modifier.testTag("approveContactNameButton"),
            ) { Text("Use new name") }
        }
    }
}
