package to.iris.chat.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class ChatListRowLayoutTest {
    @get:Rule val compose = createComposeRule()

    @Test fun previewsKeepTheRowAndFollowingChatInPlace() {
        val preview = mutableStateOf<String?>(null)
        val width = mutableStateOf(280)
        val scale = mutableStateOf(1f)
        val unread = mutableStateOf(0L)
        compose.setContent {
            CompositionLocalProvider(LocalDensity provides Density(LocalDensity.current.density, scale.value)) {
                IrisChatTheme {
                    Column(Modifier.width(width.value.dp)) {
                        IrisChatListRow(title = "Contact", preview = preview.value, timeLabel = "Now",
                            modifier = Modifier.testTag("chat-row"), unreadCount = unread.value,
                            leadingContent = { Box(Modifier.size(48.dp)) },
                            lastMessageMine = false, lastDelivery = null, onClick = {})
                        Text("Following chat", Modifier.testTag("following-chat"))
                    }
                }
            }
        }
        for (w in listOf(280, 352)) for (fontScale in listOf(1f, 1.5f)) {
            compose.runOnIdle { width.value = w; scale.value = fontScale }
            compose.waitForIdle()
            val original = compose.onNodeWithTag("chat-row").fetchSemanticsNode().boundsInRoot
            val following = compose.onNodeWithTag("following-chat").fetchSemanticsNode().boundsInRoot.top
            assertTrue(original.height > 0)
            for (text in listOf(null, "", "Short preview", "A preview that wraps onto another line. ".repeat(5),
                "One\nTwo\nThree", "Draft: A new draft", "typing…", "Hello 👋 🎉")) {
                for (count in listOf(0L, 123L)) {
                    compose.runOnIdle { preview.value = text; unread.value = count }
                    compose.waitForIdle()
                    val actual = compose.onNodeWithTag("chat-row").fetchSemanticsNode().boundsInRoot
                    assertEquals("Row height at width $w, font scale $fontScale", original.height, actual.height, 0.5f)
                    assertEquals("Following chat moved", following,
                        compose.onNodeWithTag("following-chat").fetchSemanticsNode().boundsInRoot.top, 0.5f)
                }
            }
        }
    }
}
