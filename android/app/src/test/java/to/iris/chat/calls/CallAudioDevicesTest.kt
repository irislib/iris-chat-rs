package to.iris.chat.calls

import android.telecom.CallAudioState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

@Suppress("DEPRECATION")
class CallAudioDevicesTest {
    @Test fun onlyAvailableRoutesAreOfferedAndActualRouteIsSelected() {
        val state = legacyCallAudioDevices(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_BLUETOOTH,
            CallAudioState.ROUTE_BLUETOOTH)
        assertEquals(listOf("Speaker", "Bluetooth"), state.available.map { it.name })
        assertEquals("Bluetooth", state.selected?.name)
    }

    @Test fun unpluggedHeadsetCannotRemainSelected() {
        val state = legacyCallAudioDevices(CallAudioState.ROUTE_SPEAKER or CallAudioState.ROUTE_EARPIECE,
            CallAudioState.ROUTE_BLUETOOTH)
        assertNull(state.selectedId)
        assertEquals(listOf("Phone", "Speaker"), state.available.map { it.name })
        assertEquals(emptyList<CallAudioDevice>(), legacyCallAudioDevices(0, 0).available)
    }
}
