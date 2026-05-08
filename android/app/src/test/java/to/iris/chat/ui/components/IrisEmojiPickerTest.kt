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
        assertTrue(irisEmojiMatchesQuery("😂", "Smileys", "laugh"))
        assertTrue(irisEmojiMatchesQuery("❤️", "Hearts", "love"))
    }

    @Test
    fun quickChoicesPreferPostAndRecentEmojis() {
        assertEquals(
            listOf("🔥", "😂", "❤️", "👍", "😮", "😢", "🙏"),
            irisReactionQuickChoices(
                postSuggestions = listOf("🔥"),
                recentEmojis = listOf("😂", "🔥"),
            ),
        )
    }
}
