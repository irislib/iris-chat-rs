package to.iris.chat.push

import android.content.Context
import android.content.ContextWrapper
import android.content.SharedPreferences
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.BuildConfig

@RunWith(AndroidJUnit4::class)
class CallPushProbeTest {
    @Test fun call_capture_requires_local_arm_and_remains_bounded() {
        assumeTrue(BuildConfig.DEBUG)
        // Instrumentation runs under the target UID; isolate only this test's storage.
        val target = InstrumentationRegistry.getInstrumentation().targetContext
        val preferenceName = "iris_call_probe_test_${UUID.randomUUID()}"
        val preferences = target.getSharedPreferences(preferenceName, Context.MODE_PRIVATE)
        val context = object : ContextWrapper(target) {
            override fun getSharedPreferences(name: String, mode: Int): SharedPreferences {
                require(name == "iris_push_probe")
                return preferences
            }
        }
        assertTrue(preferences.edit().clear().commit())
        try {
            PushNotificationProbe.recordCallEvent(context, "unarmed")
            assertEquals(0, PushNotificationProbe.snapshot(context).getJSONArray("call_events").length())
            PushNotificationProbe.armCallProbe(context)
            repeat(140) { index ->
                PushNotificationProbe.recordCallEvent(context, "action", callId = "call-$index")
            }
            val armed = PushNotificationProbe.snapshot(context)
            assertTrue(armed.getBoolean("call_probe_enabled"))
            assertEquals(128, armed.getJSONArray("call_events").length())
            assertEquals("call-12", armed.getJSONArray("call_events").getJSONObject(0).getString("call_id"))
            PushNotificationProbe.disarmCallProbe(context)
            PushNotificationProbe.recordCallEvent(context, "after_disarm")
            val disarmed = PushNotificationProbe.snapshot(context)
            assertFalse(disarmed.getBoolean("call_probe_enabled"))
            assertEquals(armed.getJSONArray("call_events").toString(), disarmed.getJSONArray("call_events").toString())
        } finally {
            assertTrue(target.deleteSharedPreferences(preferenceName))
        }
    }
}
