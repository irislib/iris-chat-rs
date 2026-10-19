package to.iris.chat.ui.components

import androidx.compose.ui.text.font.FontStyle
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.PersonNamePresentation
import to.iris.chat.rust.presentPersonName

fun personName(name: String, identity: String?, explicitName: String? = null): PersonNamePresentation =
    presentPersonName(name, identity.orEmpty(), explicitName)

val PersonNamePresentation.fontStyle: FontStyle
    get() = if (isFallback) FontStyle.Italic else FontStyle.Normal

fun explicitPersonName(nickname: String?, profileName: String?): String? =
    sequenceOf(nickname, profileName).mapNotNull { it?.trim()?.takeIf(String::isNotEmpty) }.firstOrNull()

fun explicitPersonName(owner: String?, state: AppState?): String? {
    val current = state?.currentChat?.takeIf { it.kind == ChatKind.DIRECT && it.chatId == owner }
    if (current != null) return explicitPersonName(current.nickname, current.profileName)
    val chat = state?.chatList?.firstOrNull { it.kind == ChatKind.DIRECT && it.chatId == owner }
    return explicitPersonName(chat?.nickname, chat?.profileName)
}

// A revision invalidates readers; all metadata/precedence stays in Rust.
internal data class UserProfiles(val manager: to.iris.chat.core.AppManager, val revision: ULong, val account: String?)
internal val LocalUserProfiles = androidx.compose.runtime.staticCompositionLocalOf<UserProfiles?> { null }

@androidx.compose.runtime.Composable
fun IrisUserName(
    ownerPubkeyHex: String,
    modifier: androidx.compose.ui.Modifier = androidx.compose.ui.Modifier,
    style: androidx.compose.ui.text.TextStyle = androidx.compose.material3.MaterialTheme.typography.bodyMedium,
) {
    val profile = LocalUserProfiles.current?.manager?.userProfile(ownerPubkeyHex)
    val name = profile?.name ?: presentPersonName("", ownerPubkeyHex, null)
    androidx.compose.material3.Text(name.name, modifier = modifier, style = style, fontStyle = name.fontStyle)
}
