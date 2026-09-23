package to.iris.chat.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.CallHistorySnapshot
import to.iris.chat.ui.theme.IrisChatTheme

@RunWith(AndroidJUnit4::class)
class CallHistoryTest {
    @get:Rule val compose = createComposeRule()

    @Test fun missedAndAnsweredCallsHaveDistinctReadableHistory() {
        compose.setContent {
            IrisChatTheme {
                Surface(color = MaterialTheme.colorScheme.background) {
                    Column {
                        CallHistoryRow(history("missed", "incoming", true, 0u))
                        CallHistoryRow(history("answered", "incoming", false, 83u))
                        CallHistoryRow(history("answered", "outgoing", true, 3_665u))
                        CallHistoryRow(history("declined", "incoming", false, 0u))
                        CallHistoryRow(history("canceled", "outgoing", false, 0u))
                        CallHistoryRow(history("answered_elsewhere", "incoming", true, 0u))
                    }
                }
            }
        }
        compose.onNodeWithText("Missed video call").assertExists()
        // Answering a video offer with voice is rendered as a voice call.
        compose.onNodeWithText("Incoming voice call").assertExists()
        compose.onNodeWithText(" · 1:23", substring = true).assertExists()
        compose.onNodeWithText("Outgoing video call").assertExists()
        compose.onNodeWithText(" · 1:01:05", substring = true).assertExists()
        compose.onNodeWithText("Declined voice call").assertExists()
        compose.onNodeWithText("Canceled voice call").assertExists()
        compose.onNodeWithText("Answered on another device").assertExists()
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val bitmap = instrumentation.uiAutomation.takeScreenshot()
        File(instrumentation.targetContext.getExternalFilesDir(null), "call-history.png").outputStream().use {
            bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }

    private fun history(outcome: String, direction: String, video: Boolean, duration: ULong) = CallHistorySnapshot(
        callId = "$direction-$outcome-$video", direction = direction, outcome = outcome, video = video,
        startedAtSecs = 1_790_158_000u, answeredAtSecs = if (outcome == "answered") 1_790_158_005u else null,
        endedAtSecs = 1_790_158_005u + duration, durationSecs = duration,
    )
}
