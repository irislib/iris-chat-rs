package to.iris.chat

import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MainActivityCallLaunchTest {
    @Test fun callIntentAfterRestartStillReturnsToThePreviousApp() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                for (id in listOf("first-call", "second-call")) {
                    val instrumentation = InstrumentationRegistry.getInstrumentation()
                    instrumentation.callActivityOnPause(activity)
                    instrumentation.callActivityOnStop(activity)
                    instrumentation.callActivityOnStart(activity)
                    try {
                        // Deliver between start and resume. ActivityScenario's STARTED
                        // state instead resumes and then pauses, missing this ordering.
                        deliverIntent(activity, callIntent(id))
                        assertEquals(id, returnAfterCallId(activity))
                    } finally {
                        instrumentation.callActivityOnResume(activity)
                    }
                }
                deliverIntent(activity, Intent(Intent.ACTION_MAIN))
                assertNull(returnAfterCallId(activity))
            }
        }
    }

    @Test fun callIntentWhileUsingTheAppKeepsTheAppOpen() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                deliverIntent(activity, callIntent("foreground-call"))
                assertNull(returnAfterCallId(activity))
            }
        }
    }

    private fun deliverIntent(activity: MainActivity, intent: Intent) {
        val original = activity.intent
        try {
            InstrumentationRegistry.getInstrumentation().callActivityOnNewIntent(activity, intent)
        } finally {
            // ActivityScenario matches lifecycle callbacks against its launch intent.
            activity.intent = original
        }
    }

    private fun callIntent(id: String) =
        Intent("to.iris.chat.SHOW_CALL").putExtra("callId", id)

    private fun returnAfterCallId(activity: MainActivity): String? =
        MainActivity::class.java.getDeclaredField("returnAfterCallId").let { field ->
            field.isAccessible = true
            field.get(activity) as String?
        }
}
