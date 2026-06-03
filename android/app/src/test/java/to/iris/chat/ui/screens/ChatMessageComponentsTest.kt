package to.iris.chat.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Test
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.rust.MessageDeliveryTraceSnapshot
import to.iris.chat.rust.MessageReactionSnapshot

class ChatMessageComponentsTest {
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
