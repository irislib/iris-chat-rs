package to.iris.chat.calls

import android.telecom.CallAudioState

data class CallAudioDevice(val id: String, val name: String)
data class CallAudioDevices(val available: List<CallAudioDevice> = emptyList(), val selectedId: String? = null) {
    val selected: CallAudioDevice? get() = available.firstOrNull { it.id == selectedId }
}

@Suppress("DEPRECATION")
internal fun legacyCallAudioDevices(supported: Int, selected: Int): CallAudioDevices {
    val routes = listOf(
        CallAudioState.ROUTE_EARPIECE to "Phone",
        CallAudioState.ROUTE_SPEAKER to "Speaker",
        CallAudioState.ROUTE_WIRED_HEADSET to "Headphones",
        CallAudioState.ROUTE_BLUETOOTH to "Bluetooth",
    ).filter { (route, _) -> supported and route != 0 }
    return CallAudioDevices(routes.map { (route, name) -> CallAudioDevice(route.toString(), name) },
        selected.toString().takeIf { id -> routes.any { it.first.toString() == id } })
}
