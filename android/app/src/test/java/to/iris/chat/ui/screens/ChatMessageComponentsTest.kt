package to.iris.chat.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import java.time.LocalDate
import java.time.ZoneId
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.CallHistorySnapshot
import org.junit.Test
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.rust.MessageDeliveryTraceSnapshot
import to.iris.chat.rust.MessageReactionSnapshot

class ChatMessageComponentsTest {
    @Test
    fun groupingUsesStableAuthorsLocalDaysAndStrictThreeMinuteGap() {
        val noon = LocalDate.now().atTime(12, 0).atZone(ZoneId.systemDefault()).toEpochSecond().toULong()
        val first = makeMessage("Hello").copy(isOutgoing = false, createdAtSecs = noon)
        val next = first.copy(id = "2", createdAtSecs = noon + 179UL)
        assertFalse(startsMessageCluster(first, next, ChatKind.GROUP))
        assertTrue(startsMessageCluster(first, next.copy(createdAtSecs = noon + 180UL), ChatKind.GROUP))
        assertTrue(startsMessageCluster(first, next.copy(createdAtSecs = noon - 1UL), ChatKind.GROUP))
        assertFalse(startsMessageCluster(first, next.copy(author = "Renamed"), ChatKind.GROUP))
        assertTrue(startsMessageCluster(first, next.copy(authorOwnerPubkeyHex = "other"), ChatKind.GROUP))
        assertTrue(startsMessageCluster(first, next.copy(isOutgoing = true), ChatKind.DIRECT))
        val reaction = listOf(MessageReactionSnapshot("👍", 1UL, false))
        assertTrue(startsMessageCluster(first.copy(reactions = reaction), next, ChatKind.DIRECT))
        assertFalse(startsMessageCluster(first, next.copy(reactions = reaction), ChatKind.DIRECT))
        val call = first.copy(call = CallHistorySnapshot("call", "incoming", "missed", false, noon, null, noon, 0UL))
        assertTrue(startsMessageCluster(first, call, ChatKind.DIRECT))
        assertTrue(startsMessageCluster(call, next, ChatKind.DIRECT))
        assertTrue(startsMessageCluster(first, next.copy(kind = ChatMessageKind.SYSTEM), ChatKind.DIRECT))
        val midnight = LocalDate.now().atStartOfDay(ZoneId.systemDefault()).toEpochSecond().toULong()
        assertTrue(startsMessageCluster(first.copy(createdAtSecs = midnight - 1UL), next.copy(createdAtSecs = midnight), ChatKind.DIRECT))
    }

    @Test
    fun footerPreservesMinuteDeliveryAndDisappearingStatusWithoutSplittingBubbles() {
        val first = makeMessage("Hello").copy(createdAtSecs = 120UL)
        val next = first.copy(id = "2", createdAtSecs = 121UL)
        assertFalse(showsMessageFooter(first, next, ChatKind.DIRECT))
        assertTrue(showsMessageFooter(first, null, ChatKind.DIRECT))
        assertTrue(showsMessageFooter(first, next.copy(createdAtSecs = 180UL), ChatKind.DIRECT))
        assertFalse(startsMessageCluster(first, next.copy(createdAtSecs = 180UL), ChatKind.DIRECT))
        assertTrue(showsMessageFooter(first, next.copy(delivery = DeliveryState.SEEN), ChatKind.DIRECT))
        for (status in listOf(DeliveryState.QUEUED, DeliveryState.PENDING, DeliveryState.FAILED)) {
            assertTrue(showsMessageFooter(first.copy(delivery = status), next.copy(delivery = status), ChatKind.DIRECT))
        }
        assertTrue(showsMessageFooter(first.copy(expiresAtSecs = 1000UL), next, ChatKind.DIRECT))
    }

    @Test
    fun postReactionSuggestionsIncludeExistingMessageEmoji() {
        val reactions =
            listOf(
                MessageReactionSnapshot(emoji = "🔥", count = 1UL, reactedByMe = true),
                MessageReactionSnapshot(emoji = "🔥", count = 2UL, reactedByMe = true),
                MessageReactionSnapshot(emoji = "😂", count = 1UL, reactedByMe = false),
            )

        assertEquals(listOf("🔥", "😂"), postReactionSuggestionEmojis(reactions))
    }

    @Test
    fun jumbomojiCountOnlyAcceptsUpToFiveEmojiIgnoringWhitespace() {
        assertEquals(1, jumbomojiCount("🔥"))
        assertEquals(2, jumbomojiCount("🔥 😂"))
        assertEquals(1, jumbomojiCount("👨‍👩‍👧‍👦"))
        assertEquals(5, jumbomojiCount("😀😃😄😁😆"))
        assertEquals(0, jumbomojiCount("😀😃😄😁😆😅"))
        assertEquals(0, jumbomojiCount("nice 🔥"))
    }

    @Test
    fun messageUrlMatchesAcceptBareDomainsWithPaths() {
        assertEquals(
            listOf(
                MessageUrlMatch(
                    range = 6..24,
                    visible = "github.com/username",
                    url = "https://github.com/username",
                ),
            ),
            messageUrlMatches("visit github.com/username"),
        )
    }

    @Test
    fun messageUrlMatchesTrimTrailingPunctuation() {
        assertEquals(
            listOf(
                MessageUrlMatch(
                    range = 1..16,
                    visible = "example.com/path",
                    url = "https://example.com/path",
                ),
            ),
            messageUrlMatches("(example.com/path)."),
        )
    }

    @Test
    fun messageUrlMatchesKeepSchemedUrlsAndSkipEmails() {
        assertEquals(
            listOf(
                MessageUrlMatch(
                    range = 25..44,
                    visible = "https://iris.to/chat",
                    url = "https://iris.to/chat",
                ),
            ),
            messageUrlMatches("mail me@example.com then https://iris.to/chat"),
        )
    }

    @Test
    fun forwardableMessageTextStripsQuotedReplyAndKeepsAttachments() {
        val attachment =
            MessageAttachmentSnapshot(
                nhash = "nhash1photo",
                filename = "photo.jpg",
                filenameEncoded = "photo.jpg",
                htreeUrl = "htree://nhash1photo/photo.jpg",
                isImage = true,
                isVideo = false,
                isAudio = false,
            )
        val message =
            makeMessage(
                body = "${ReplyMessagePrefix}Alice: old text\n\nnew text",
                attachments = listOf(attachment),
            )

        assertEquals(
            "new text\nhtree://nhash1photo/photo.jpg",
            forwardableMessageText(message),
        )
    }

    @Test
    fun forwardableMessageTextCanBeOnlyAttachment() {
        val attachment =
            MessageAttachmentSnapshot(
                nhash = "nhash1clip",
                filename = "clip.mp4",
                filenameEncoded = "clip.mp4",
                htreeUrl = "htree://nhash1clip/clip.mp4",
                isImage = false,
                isVideo = true,
                isAudio = false,
            )

        assertEquals(
            "htree://nhash1clip/clip.mp4",
            forwardableMessageText(makeMessage(body = "", attachments = listOf(attachment))),
        )
    }

    private fun makeMessage(
        body: String,
        attachments: List<MessageAttachmentSnapshot> = emptyList(),
    ): ChatMessageSnapshot =
        ChatMessageSnapshot(
            id = "1",
            chatId = "chat-1",
            kind = ChatMessageKind.USER,
            author = "owner-hex",
            authorOwnerPubkeyHex = "owner-hex",
            authorPictureUrl = null,
            body = body,
            attachments = attachments,
            reactions = emptyList(),
            reactors = emptyList(),
            isOutgoing = true,
            createdAtSecs = 1u,
            expiresAtSecs = null,
            delivery = DeliveryState.SENT,
            recipientDeliveries = emptyList(),
            deliveryTrace =
                MessageDeliveryTraceSnapshot(
                    outerEventIds = emptyList(),
                    pendingRelayEventIds = emptyList(),
                    queuedProtocolTargets = emptyList(),
                    transportChannels = emptyList(),
                    lastTransportError = null,
                ),
            sourceEventId = null,
        )
}
