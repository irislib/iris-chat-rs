package to.iris.chat.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.DeviceAuthorizationState

class ChatLinkTest {
    private val hex = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    private val npub = "npub10xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqpkge6d"
    private val profile = "nprofile1qqs8n0nx0muaewav2ksx99wwsu9swq5mlndjmn3gm9vl9q2mzmup0xqq60rzu"

    @Test
    fun bothSchemesUseProductionIdentityNormalization() {
        for (scheme in listOf("https", "irischat")) {
            assertEquals(AppAction.CreateChat(hex), parseChatLink("$scheme://chat.iris.to/$hex"))
            assertEquals(AppAction.CreateChat(npub), parseChatLink("$scheme://chat.iris.to/#/$npub"))
            assertEquals(AppAction.CreateChat(npub), parseChatLink("$scheme://chat.iris.to/$profile"))
            assertEquals(AppAction.CreateChat(npub), parseChatLink("$scheme://chat.iris.to/#nostr%3A$npub"))
        }
    }

    @Test
    fun invitePathsAndPrivateFragmentsKeepTheirOriginalEncoding() {
        for (suffix in listOf(
            "/invite/token%2Fpart?source=web#secret%2Bvalue+tail",
            "/#/invite/token%2Fpart",
            "/#%7B%22ephemeralKey%22%3A%22public%22%2C%22sharedSecret%22%3A%22fixture%2B%22%7D",
        )) {
            val expected = AppAction.AcceptInvite("https://chat.iris.to$suffix")
            assertEquals(expected, parseChatLink("irischat://chat.iris.to$suffix"))
            assertEquals(expected, parseChatLink("https://chat.iris.to$suffix"))
        }
    }

    @Test
    fun onlyExactCanonicalAuthorityAndSupportedSchemesAreAccepted() {
        for (url in listOf(
            "http://chat.iris.to/$hex",
            "irischat://other.example/$hex",
            "irischat://chat.iris.to.other.example/$hex",
            "irischat://user@chat.iris.to/$hex",
            "irischat://chat.iris.to:443/$hex",
            "irischat:chat.iris.to/$hex",
            "irischat://chat.iris.to/%zz",
            "irischat://chat.iris.to/not-a-user",
            "irischat://chat.iris.to/invite/",
            "",
        )) assertNull(url, parseChatLink(url))
        assertEquals(AppAction.CreateChat(hex), parseChatLink("IRISCHAT://CHAT.IRIS.TO/$hex"))
    }

    @Test
    fun firstInstallAndDeviceApprovalRetainLinkUntilAuthorizedOnce() {
        val pending = PendingChatLink()
        pending.offer("irischat://chat.iris.to/$hex")
        assertNull(pending.takeWhenAuthorized(null))
        assertNull(pending.takeWhenAuthorized(DeviceAuthorizationState.AWAITING_APPROVAL))
        assertEquals(AppAction.CreateChat(hex), pending.takeWhenAuthorized(DeviceAuthorizationState.AUTHORIZED))
        assertNull(pending.takeWhenAuthorized(DeviceAuthorizationState.AUTHORIZED))
    }

    @Test
    fun latestValidLinkWinsAndInvalidIntentCannotEraseIt() {
        val pending = PendingChatLink()
        pending.offer("https://chat.iris.to/$hex")
        pending.offer("irischat://chat.iris.to/#/invite/fixture")
        pending.offer("irischat://other.example/$hex")
        assertEquals(
            AppAction.AcceptInvite("https://chat.iris.to/#/invite/fixture"),
            pending.takeWhenAuthorized(DeviceAuthorizationState.AUTHORIZED),
        )
    }

    @Test
    fun logoutResetAndRevocationDiscardPendingLinks() {
        val pending = PendingChatLink()
        pending.offer("https://chat.iris.to/$hex")
        pending.clear()
        assertNull(pending.takeWhenAuthorized(DeviceAuthorizationState.AUTHORIZED))
        pending.offer("https://chat.iris.to/$hex")
        assertNull(pending.takeWhenAuthorized(DeviceAuthorizationState.REVOKED))
        assertNull(pending.takeWhenAuthorized(DeviceAuthorizationState.AUTHORIZED))
    }
}
