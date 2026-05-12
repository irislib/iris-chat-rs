package to.iris.chat.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Test
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
}
