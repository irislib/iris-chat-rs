package to.iris.chat.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.buildLargeTestAppState

class MessageEditingTest {
    @Test fun editingPreservesQuoteButReplacesOnlyTheBody() {
        assertEquals("↩ Alice: See you soon\n\nAfter",
            editedMessageText("↩ Alice: See you soon\n\nBefore", "After"))
        assertEquals("After", editedMessageText("Before\n\nsecond paragraph", "After"))
        assertEquals("After", editedMessageText("↩ malformed", "After"))
    }

    @Test fun onlySentOwnTextMessagesCanBeEdited() {
        val message = buildLargeTestAppState(1u, 0u, 1u).currentChat!!.messages.first()
            .copy(kind = ChatMessageKind.USER, isOutgoing = true, delivery = DeliveryState.SENT, attachments = emptyList(), call = null)
        assertTrue(canEditMessage(message))
        assertFalse(canEditMessage(message.copy(isOutgoing = false)))
        assertFalse(canEditMessage(message.copy(delivery = DeliveryState.QUEUED)))
        assertFalse(canEditMessage(message.copy(delivery = DeliveryState.PENDING)))
        assertFalse(canEditMessage(message.copy(deletedForEveryone = true)))
        assertEquals("Message deleted", replySnippet(message.copy(deletedForEveryone = true)))
    }
}
