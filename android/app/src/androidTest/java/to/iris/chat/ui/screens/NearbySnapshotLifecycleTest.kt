package to.iris.chat.ui.screens

import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.nearby.IrisNearbyService

@RunWith(AndroidJUnit4::class)
class NearbySnapshotLifecycleTest {
    @get:Rule val compose = createComposeRule()

    @Test fun snapshotPollingPausesWhileStoppedAndRefreshesOnResume() {
        compose.mainClock.autoAdvance = false
        lateinit var owner: Owner
        val service = IrisNearbyService(InstrumentationRegistry.getInstrumentation().targetContext)
        compose.runOnUiThread { owner = Owner().apply { registry.currentState = Lifecycle.State.RESUMED } }
        compose.setContent {
            CompositionLocalProvider(LocalLifecycleOwner provides owner) {
                val snapshot by rememberNearbySnapshotState(service)
                Text(snapshot.status)
            }
        }
        compose.mainClock.advanceTimeBy(100)
        compose.onNodeWithText("Off").assertExists()
        compose.runOnUiThread {
            owner.registry.currentState = Lifecycle.State.CREATED
            service.setFipsBluetoothVisible(true)
        }
        compose.mainClock.advanceTimeBy(5_000)
        compose.onNodeWithText("Off").assertExists()
        compose.runOnUiThread { owner.registry.currentState = Lifecycle.State.RESUMED }
        compose.mainClock.advanceTimeBy(100)
        compose.onNodeWithText("Visible").assertExists()
    }

    private class Owner : LifecycleOwner {
        val registry = LifecycleRegistry(this)
        override val lifecycle: Lifecycle get() = registry
    }
}
