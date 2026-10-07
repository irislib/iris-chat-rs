package to.iris.chat.nearby

import android.Manifest
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.buildLargeTestAppState

@RunWith(AndroidJUnit4::class)
class NearbyBackgroundLifecycleTest {
    @Test fun restoredAccountEnablesLocalDiscoveryWithoutAnActivity() = runBlocking {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        if (Build.VERSION.SDK_INT >= 33) {
            instrumentation.uiAutomation.grantRuntimePermission(context.packageName,
                Manifest.permission.NEARBY_WIFI_DEVICES)
        }
        val saved = buildLargeTestAppState(1u, 0u, 0u)
        saved.account!!.authorizationState = DeviceAuthorizationState.AUTHORIZED
        saved.preferences.nearbyEnabled = true
        saved.preferences.nearbyLanEnabled = true
        saved.preferences.nearbyBluetoothEnabled = false
        val states = MutableStateFlow(saved.copy(account = null))
        val service = IrisNearbyService(context)
        val observer = service.observeAppState(states, this)
        try {
            assertFalse(service.snapshot.localNetworkVisible)
            states.value = saved
            withTimeout(5_000) { while (!service.snapshot.localNetworkVisible) delay(10) }
            assertTrue(service.snapshot.localNetworkPermissionGranted)

            states.value = saved.copy(account = saved.account!!.copy(
                authorizationState = DeviceAuthorizationState.REVOKED))
            withTimeout(5_000) { while (service.snapshot.localNetworkVisible) delay(10) }

            states.value = saved
            withTimeout(5_000) { while (!service.snapshot.localNetworkVisible) delay(10) }
            states.value = saved.copy(account = null)
            withTimeout(5_000) { while (service.snapshot.localNetworkVisible) delay(10) }
        } finally {
            observer.cancel()
            service.setLocalNetworkVisible(false)
        }
    }
}
