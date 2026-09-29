package to.iris.chat.ui.screens

import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import android.graphics.Bitmap
import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.AppAction
import to.iris.chat.ui.components.rememberChatMuteAction
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class ChatMuteOptionsTest {
    @get:Rule val compose = createComposeRule()

    @Test fun durationDispatchesDeadlineAndDismissesWithoutChangingAnotherChat() {
        val actions = mutableListOf<AppAction>()
        compose.setContent { IrisChatTheme(darkTheme = false) {
            TextButton(onClick = rememberChatMuteAction("test-chat", false, actions::add)) { Text("Mute") }
        } }
        compose.onNodeWithText("Mute").performClick()
        compose.onNodeWithText("1 week").assertExists()
        compose.waitForIdle()
        Thread.sleep(350) // Allow the platform dialog window enter animation to settle before capture.
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.uiAutomation.takeScreenshot()?.let { bitmap ->
            File(instrumentation.targetContext.cacheDir, "timed-mute-options.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
        val before = (System.currentTimeMillis() / 1000).toULong()
        compose.onNodeWithText("1 hour").performClick()
        compose.onNodeWithText("Always").assertDoesNotExist()
        compose.runOnIdle {
            val action = actions.single() as AppAction.SetChatMuteUntil
            assertEquals("test-chat", action.chatId)
            assertTrue(action.untilSecs in (before + 3600uL)..((System.currentTimeMillis() / 1000).toULong() + 3600uL))
        }
    }

    @Test fun unmuteTargetsOnlyTheDisplayedChat() {
        val actions = mutableListOf<AppAction>()
        compose.setContent { IrisChatTheme(darkTheme = false) {
            TextButton(onClick = rememberChatMuteAction("group:test", true, actions::add)) { Text("Options") }
        } }
        compose.onNodeWithText("Options").performClick()
        compose.onNodeWithText("Unmute").performClick()
        compose.runOnIdle { assertEquals(AppAction.SetChatMuted("group:test", false), actions.single()) }
    }
}
