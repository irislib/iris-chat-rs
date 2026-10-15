package to.iris.chat.ui.screens

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import to.iris.chat.rust.messageMentions

internal fun linkedMentionMessageAnnotatedString(text: String, color: Color, onMention: (String) -> Unit, names: Map<String, String> = emptyMap()): AnnotatedString =
    buildAnnotatedString {
        var cursor = 0
        for (mention in messageMentions(text)) {
            val start = mention.start.toInt()
            append(linkedPlainMessageAnnotatedString(text.substring(cursor, start), color))
            val labelStart = length
            append(names[mention.ownerPubkeyHex]?.let { "@$it" } ?: mention.label)
            addLink(LinkAnnotation.Clickable(mention.ownerPubkeyHex,
                TextLinkStyles(SpanStyle(color = color, fontWeight = FontWeight.SemiBold, textDecoration = TextDecoration.Underline))) {
                onMention(mention.ownerPubkeyHex)
            }, labelStart, length)
            cursor = start + mention.length.toInt()
        }
        append(linkedPlainMessageAnnotatedString(text.substring(cursor), color))
    }
