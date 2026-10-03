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
