package to.iris.chat.ui.screens

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration

// Signal-style link styling: force the bubble's body text colour so URL hue
// stays stable between incoming and outgoing bubbles, then underline.
internal fun linkedPlainMessageAnnotatedString(text: String, color: Color, onPayment: ((String) -> Unit)? = null): AnnotatedString =
    buildAnnotatedString {
        val linkStyle = SpanStyle(color = color, textDecoration = TextDecoration.Underline)
        var index = 0
        for (match in messageUrlMatches(text)) {
            append(text.substring(index, match.range.first))
            val visible = match.visible
            val start = length
            append(visible)
            addLink(
                LinkAnnotation.Url(
                    url = match.url,
                    styles = TextLinkStyles(style = linkStyle),
                    linkInteractionListener = if (onPayment != null && (match.url.startsWith("cashu:") || match.url.startsWith("lightning:"))) {
                        androidx.compose.ui.text.LinkInteractionListener { onPayment(match.url) }
                    } else null,
                ),
                start,
                length,
            )
            index = match.range.last + 1
        }
        if (index < text.length) {
            append(text.substring(index))
        }
    }

private fun trimTrailingUrlPunctuation(value: String): String =
    value.trimEnd('.', ',', ';', ':', '!', '?', ')', ']')

private fun normalizedMessageUrl(value: String): String =
    if (value.startsWith("http://", ignoreCase = true) ||
        value.startsWith("https://", ignoreCase = true)
    ) {
        value
    } else {
        "https://$value"
    }

internal data class MessageUrlMatch(
    val range: IntRange,
    val visible: String,
    val url: String,
)

internal fun messageUrlMatches(text: String): List<MessageUrlMatch> {
    val payments = to.iris.chat.rust.messagePaymentLinks(text).map { payment ->
        MessageUrlMatch(
            range = payment.start.toInt() until (payment.start + payment.length).toInt(),
            visible = payment.label,
            url = payment.uri,
        )
    }
    val links = MessageUrlRegex.findAll(text).mapNotNull { match ->
        val group = match.groups[1] ?: return@mapNotNull null
        val visible = trimTrailingUrlPunctuation(group.value)
        if (visible.isEmpty()) {
            return@mapNotNull null
        }
        val end = group.range.first + visible.length - 1
        if (payments.any { it.range.first <= end && it.range.last >= group.range.first }) return@mapNotNull null
        MessageUrlMatch(
            range = group.range.first..end,
            visible = visible,
            url = normalizedMessageUrl(visible),
        )
    }.toList()
    return (payments + links).sortedBy { it.range.first }
}

private val MessageUrlRegex =
    Regex(
        """(?i)(?:^|(?<=[\s(\[{<]))((?:https?://|www\.)[^\s<]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}(?::[0-9]{2,5})?(?:/[^\s<]*)?)""",
    )


internal fun copyableCashuTokens(text: String): List<String> =
    to.iris.chat.rust.messagePaymentLinks(text).filter { it.cashu != null }.map { it.uri.removePrefix("cashu:") }
