package to.iris.chat.ui.components

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class IrisEmojiPickerTest {
    @Test
    fun searchMatchesUnicodeEmojiNames() {
        assertTrue(irisEmojiMatchesQuery("👍", "Hands", "thumbs up"))
        assertTrue(irisEmojiMatchesQuery("🍕", "Food", "pizza"))
        assertFalse(irisEmojiMatchesQuery("🍕", "Food", "thumbs up"))
    }

    @Test
    fun searchMatchesCommonAliases() {
        val aliases = mapOf("😂" to "laugh laughing lol haha", "❤️" to "love heart red")
        assertTrue(irisEmojiMatchesQuery("😂", "Smileys", "laugh", aliases))
        assertTrue(irisEmojiMatchesQuery("❤️", "Hearts", "love", aliases))
    }

    @Test
    fun quickChoicesUseBasicReactionSet() {
        assertEquals(listOf("❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"), irisReactionQuickChoices())
    }

    @Test
    fun emptyPickerShowsThisMessageSectionForMessageEmoji() {
        val sections =
            irisEmojiPickerSections(
                query = "",
                suggestedEmojis = listOf("🔥", "🔥", "😂"),
                recentEmojis = listOf("😂", "🙏"),
            )

        assertEquals("This message", sections.first().first)
        assertEquals(listOf("🔥", "😂"), sections.first().second)
        assertEquals("Recent", sections[1].first)
        assertEquals(listOf("🙏"), sections[1].second)
    }
}
