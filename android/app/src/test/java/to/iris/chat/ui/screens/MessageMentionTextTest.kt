package to.iris.chat.ui.screens

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.LinkAnnotation
import org.junit.Assert.*
import org.junit.Test
import to.iris.chat.rust.formatMessageMention
import to.iris.chat.rust.messageMentions

class MessageMentionTextTest {
    @Test fun renamedMentionsKeepIdentityAndOrdinaryLinks() {
        val owner = "11".repeat(32)
        val token = formatMessageMention(owner, "Old name")
        val source = "Hello $token, https://iris.to!"
        var selected: String? = null
        val rendered = linkedMentionMessageAnnotatedString(source, Color.White, { selected = it }, mapOf(owner to "New name"))
        assertEquals("Hello @New name, https://iris.to!", rendered.text)
        val links = rendered.getLinkAnnotations(0, rendered.length).map { it.item }
        val mention = links.filterIsInstance<LinkAnnotation.Clickable>().single()
        mention.linkInteractionListener?.onClick(mention)
        assertEquals(owner, selected)
        assertEquals("https://iris.to", links.filterIsInstance<LinkAnnotation.Url>().single().url)
        assertEquals(owner, messageMentions(source).single().ownerPubkeyHex)
    }

    @Test fun unstructuredAndMalformedNamesHaveNoMentionAction() {
        val source = "@Alex [@Alex](nostr:not-a-valid-key)"
        val rendered = linkedMentionMessageAnnotatedString(source, Color.White, {})
        assertEquals(source, rendered.text)
        assertTrue(rendered.getLinkAnnotations(0, rendered.length).isEmpty())
    }
}
