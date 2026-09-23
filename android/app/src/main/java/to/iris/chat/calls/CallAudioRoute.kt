package to.iris.chat.calls

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build

/** Standalone media tests own focus; production calls use Telecom's focus. */
internal class CallAudioRoute(context: Context, private val telecomManaged: Boolean = false, failed: () -> Unit) : AutoCloseable {
    private val manager = context.getSystemService(AudioManager::class.java)
    private val oldMode = manager.mode
    @Suppress("DEPRECATION") private val oldSpeaker = manager.isSpeakerphoneOn
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
        .setOnAudioFocusChangeListener { if (it == AudioManager.AUDIOFOCUS_LOSS) failed() }.build()
    private var started = false

    fun start(speaker: Boolean) {
        if (!telecomManaged) check(manager.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
        started = true
        if (!telecomManaged) manager.mode = AudioManager.MODE_IN_COMMUNICATION
        setSpeaker(speaker)
    }

    @Suppress("DEPRECATION")
    fun setSpeaker(enabled: Boolean) {
        if (telecomManaged) { IrisConnectionService.setSpeaker(enabled); return }
        if (Build.VERSION.SDK_INT >= 31) {
            val devices = manager.availableCommunicationDevices
            val external = devices.firstOrNull { it.type in setOf(AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
                AudioDeviceInfo.TYPE_BLE_HEADSET, AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_USB_HEADSET) }
            val selected = if (enabled) devices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
                else external ?: devices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE }
            if (selected != null) manager.setCommunicationDevice(selected) else manager.clearCommunicationDevice()
        } else manager.isSpeakerphoneOn = enabled
    }

    @Suppress("DEPRECATION")
    override fun close() {
        if (!started) return
        started = false
        if (telecomManaged) return
        if (Build.VERSION.SDK_INT >= 31) manager.clearCommunicationDevice() else manager.isSpeakerphoneOn = oldSpeaker
        manager.mode = oldMode
        manager.abandonAudioFocusRequest(focus)
    }
}
