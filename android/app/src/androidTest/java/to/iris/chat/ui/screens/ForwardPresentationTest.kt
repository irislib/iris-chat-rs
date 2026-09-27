package to.iris.chat.ui.screens

import android.graphics.Bitmap
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.buildLargeTestAppState
import to.iris.chat.rust.formatForwardedMessage
import to.iris.chat.ui.theme.IrisChatTheme

class ForwardPresentationTest {
    @get:Rule val compose = createComposeRule()

    @Test fun forwardedBodyIsVisibleOnBothSidesWithoutOriginalAuthor() {
        val source = buildLargeTestAppState(1u, 0u, 1u).currentChat!!.messages.first()
            .copy(body = "Bring a picnic blanket.", kind = ChatMessageKind.USER, call = null,
                author = "Original private sender", attachments = emptyList(), expiresAtSecs = null)
        val text = formatForwardedMessage(forwardableMessageText(source))
        val forwarded = source.copy(body = text, author = "Alex")
        assertEquals("Forwarded:\n\nBring a picnic blanket.", text)
        assertEquals(text, formatForwardedMessage(forwardableMessageText(forwarded)))
        compose.setContent {
            IrisChatTheme(darkTheme = false) {
                Column(Modifier.statusBarsPadding().padding(16.dp)) {
                    Text("Forwarded message")
                    for (outgoing in listOf(false, true)) {
                        MessageBubble(
                            message = forwarded.copy(id = "forward-$outgoing", isOutgoing = outgoing),
                            chatKind = ChatKind.DIRECT, isFirstInCluster = true, isLastInCluster = true,
                            reactions = emptyList(), onReply = {}, onForward = {}, onReact = {}, onDelete = {},
                            onScrollToQuote = {}, downloadAttachment = { null }, onOpenImage = { _, _ -> },
                        )
                    }
                }
            }
        }
        compose.onAllNodesWithText(text).assertCountEquals(2)
        compose.onAllNodesWithText("Original private sender").assertCountEquals(0)
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        File(instrumentation.targetContext.getExternalFilesDir("screenshots"), "forwarded-bubbles.png")
            .outputStream().use {
                instrumentation.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, it)
            }
    }
}
