package to.iris.chat.core

import java.net.URI
import java.net.URLDecoder
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput

internal fun parseChatLink(input: String): AppAction? {
    val raw = input.trim()
    val uri = runCatching { URI(raw) }.getOrNull() ?: return null
    if (uri.scheme?.lowercase() !in setOf("https", "irischat") ||
        !uri.rawAuthority.equals("chat.iris.to", ignoreCase = true)
    ) return null

    // Change only the scheme: private invitation bytes must survive unchanged.
    val canonical = "https" + raw.substring(raw.indexOf(':'))
    val path = uri.rawPath.orEmpty().split('/').filter(String::isNotBlank).map(::decodeLinkPart)
    val fragment = decodeLinkPart(uri.rawFragment.orEmpty())
    val fragmentSegments = fragment.trim().removePrefix("/").split('/').filter(String::isNotBlank)
    val isInvite =
        (path.firstOrNull().equals("invite", ignoreCase = true) && path.size >= 2) ||
            (fragmentSegments.firstOrNull().equals("invite", ignoreCase = true) && fragmentSegments.size >= 2) ||
            (fragment.contains("\"ephemeralKey\"") && fragment.contains("\"sharedSecret\""))
    if (isInvite) return AppAction.AcceptInvite(canonical)

    for (candidate in listOfNotNull(path.lastOrNull(), fragmentSegments.firstOrNull(), fragment)) {
        val normalized = normalizePeerInput(candidate)
        if (normalized.isNotBlank() && isValidPeerInput(normalized)) return AppAction.CreateChat(normalized)
    }
    return null
}

private fun decodeLinkPart(value: String): String =
    URLDecoder.decode(value.replace("+", "%2B"), Charsets.UTF_8.name())

/** An Activity can be recreated during onboarding; the AppManager retains its link. */
internal class PendingChatLink {
    private var action: AppAction? = null
    private var notificationOwner: String? = null

    @Synchronized
    fun offer(input: String) {
        parseChatLink(input)?.let { action = it; notificationOwner = null }
    }

    @Synchronized
    fun offerChat(chatId: String, owner: String?) {
        val targetOwner = owner?.trim()?.takeIf(String::isNotEmpty) ?: return
        chatId.trim().takeIf(String::isNotEmpty)?.let {
            action = AppAction.OpenChat(it)
            notificationOwner = targetOwner
        }
    }

    @Synchronized
    fun takeWhenAuthorized(state: DeviceAuthorizationState?, owner: String? = null): AppAction? {
        // Keep a cold-start tap while secrets load, but never carry a banner
        // into another account (including after logout and a fresh login).
        if (notificationOwner != null && owner != null && notificationOwner != owner) {
            clear()
            return null
        }
        return when (state) {
            DeviceAuthorizationState.AUTHORIZED -> {
                if (notificationOwner != null && owner == null) null
                else action.also { clear() }
            }
            DeviceAuthorizationState.REVOKED -> null.also { clear() }
            else -> null
        }
    }

    @Synchronized
    fun clear() { action = null; notificationOwner = null }
}
