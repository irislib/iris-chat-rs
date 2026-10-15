package to.iris.chat.ui.screens

import org.junit.Assert.*
import org.junit.Test

class EmojiCompletionTest {
    @Test
    fun literalColonsAndSelectionsAreNotCompletionTokens() {
        for (text in listOf("https://iris.to/:sm", "12:30", "word:sm", ":smile::", "::smile:", ":".repeat(100_000))) {
            assertNull(text.take(40), ComposerEmojiToken.find(text, text.length))
        }
        assertNull(ComposerEmojiToken.find(":sm", 1, 3))
        assertNull(ComposerEmojiToken.find(":sm", 99))
        assertNull(ComposerEmojiToken.find(":sm" + "x".repeat(100_000), 3))
    }

    @Test
    fun replacementUsesUtf16AndIncludesClosedSuffix() {
        val text = "👩🏽‍💻 :smile: after"
        val token = ComposerEmojiToken.find(text, "👩🏽‍💻 :sm".length)!!
        assertEquals("sm", token.query)
        assertEquals("👩🏽‍💻 😄 after", text.replaceRange(token.start, token.end, "😄"))
        assertNull(ComposerEmojiToken.find(text, 1))
        assertTrue(ComposerEmojiToken.find(":fire:", 6)!!.closed)
    }

    @Test
    fun exactShortcodesRankFirstAndOneCharacterDoesNotOpen() {
        val entries = listOf(ComposerEmojiEntry("😃", listOf("smiley")), ComposerEmojiEntry("😄", listOf("smile")), ComposerEmojiEntry("👍", listOf("thumbsup", "+1", "thumbs_up")))
        assertTrue(ComposerEmojiEntry.search("s", entries).isEmpty())
        assertEquals("😄", ComposerEmojiEntry.search("smile", entries).first().emoji)
        assertEquals("👍", ComposerEmojiEntry.search("+1", entries).first().emoji)
        assertEquals("👍", ComposerEmojiEntry.search("thumbs-up", entries).first().emoji)
        assertTrue(ComposerEmojiEntry.search("not_an_emoji", entries).isEmpty())
    }
}
