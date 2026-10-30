package to.iris.chat.ui.screens

import org.junit.Assert.assertEquals
import org.junit.Test

class ContactNicknameTest {
    @Test fun emptyAndWhitespaceOnlyDraftsNormalizeToEmpty() {
        assertEquals("", normalizeContactNickname(""))
        assertEquals("", normalizeContactNickname(" \t\r\n\u0085\u00a0\u2003\u202f\u3000"))
    }

    @Test fun collapsesAsciiAndUnicodeWhitespaceWithoutLeadingOrTrailingSpaces() {
        assertEquals("Alice Bob", normalizeContactNickname("  Alice\t \r\nBob  "))
        assertEquals("Alice 😀", normalizeContactNickname("\u0085\u00a0Alice\u2003\u202f😀\u0085\u3000"))
        val separators = listOf(
            '\t', '\n', '\u000b', '\u000c', '\r', ' ', '\u0085', '\u00a0', '\u1680',
            '\u2000', '\u2001', '\u2002', '\u2003', '\u2004', '\u2005', '\u2006',
            '\u2007', '\u2008', '\u2009', '\u200a', '\u2028', '\u2029', '\u202f', '\u205f', '\u3000',
        )
        for (separator in separators) {
            assertEquals("U+${separator.code.toString(16)}", "A B",
                normalizeContactNickname("${separator}A${separator}${separator}B$separator"))
        }
    }

    @Test fun preservesNonWhitespaceAndUnicodeScalarValues() {
        for (character in listOf('\u001c', '\u001d', '\u001e', '\u001f', '\u180e', '\u200b', '\ufeff')) {
            val value = "${character}A${character}B$character"
            assertEquals(value, normalizeContactNickname(value))
        }
        val value = "👩🏽‍💻 e\u0301 😀"
        assertEquals(value, normalizeContactNickname(value))
        val boundary = "😀".repeat(80)
        val normalized = normalizeContactNickname("\u0085$boundary\u0085")
        assertEquals(boundary, normalized)
        assertEquals(80, normalized.codePointCount(0, normalized.length))
    }
}
