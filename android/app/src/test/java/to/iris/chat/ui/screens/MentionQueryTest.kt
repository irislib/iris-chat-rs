package to.iris.chat.ui.screens

import androidx.compose.ui.text.TextRange
import org.junit.Assert.*
import org.junit.Test

class MentionQueryTest {
    @Test fun bareAtAndNamesWithSpacesOpenImmediately() {
        assertEquals(MentionQuery(0, 1, ""), mentionQuery("@", TextRange(1)))
        assertEquals("Alex S", mentionQuery("👋 @Alex S", TextRange("👋 @Alex S".length))!!.query)
        assertEquals("Åsa", mentionQuery("@Åsa", TextRange(4))!!.query)
    }
    @Test fun emailSelectionAndOversizedTokensStayLiteral() {
        for (text in listOf("person@example", "@@Alex", "@" + "x".repeat(100_000)))
            assertNull(mentionQuery(text, TextRange(text.length)))
        assertNull(mentionQuery("@Alex", TextRange(1, 3)))
        assertNull(mentionQuery("@Alex", TextRange(100)))
        assertNull(mentionQuery("@", TextRange(0)))
    }
    @Test fun replacingAtCaretIncludesTheRestOfTheName() {
        val text = "👩🏽‍💻 @Alex after"
        val token = mentionQuery(text, TextRange("👩🏽‍💻 @Al".length))!!
        assertEquals("👩🏽‍💻 @Chosen after", text.replaceRange(token.start, token.end, "@Chosen"))
    }
}
