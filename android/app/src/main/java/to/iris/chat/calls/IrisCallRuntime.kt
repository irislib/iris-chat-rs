package to.iris.chat.calls

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioManager
import android.media.Ringtone
import android.media.RingtoneManager
import android.os.Build
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.CallSnapshot

class IrisCallRuntime(private val context: Context, private val app: AppManager, private val scope: CoroutineScope) {
    private val mutableRemoteVideo = MutableStateFlow<CallVideoStream?>(null)
    private val mutableLocalVideo = MutableStateFlow<CallVideoStream?>(null)
    private val mutableError = MutableStateFlow<String?>(null)
    private val mutableSpeaker = MutableStateFlow(false)
    private val mutableAnswerRequest = MutableStateFlow<String?>(null)
    val remoteVideo = mutableRemoteVideo.asStateFlow()
    val localVideo = mutableLocalVideo.asStateFlow()
    val error = mutableError.asStateFlow()
    val speaker = mutableSpeaker.asStateFlow()
    val answerRequest = mutableAnswerRequest.asStateFlow()
    private var ringtone: Ringtone? = null
    @Volatile private var current: CallSnapshot? = null
    @Volatile private var audio: CallAudio? = null
    @Volatile private var video: CallVideoMedia? = null
    private var connectedSent = false
    private var audioRoute: CallAudioRoute? = null
    @Volatile private var mediaCallId: String? = null
    private var announcedId: String? = null
    @Volatile private var failedCallId: String? = null
    private var serviceReady = false

    init {
        app.setCallMediaReceiver(::receive)
        scope.launch(Dispatchers.Main.immediate) {
            combine(app.call, app.preferences) { call, _ -> call }.collect { update(it) }
        }
    }

    private fun update(call: CallSnapshot?) {
        current = call
        if (call == null || call.phase == "ended") {
            ringtone?.stop(); ringtone = null
            mutableAnswerRequest.value = null
            stopMedia()
            IrisConnectionService.finishCurrent()
            context.stopService(Intent(context, IrisCallService::class.java))
            announcedId = null
            serviceReady = false
            return
        }
        if (announcedId != call.callId) {
            stopMedia()
            failedCallId = null
            mutableError.value = null
            mutableSpeaker.value = call.video
            announcedId = call.callId
            IrisConnectionService.announce(context, call)
            if (call.phase == "incoming" && context.getSystemService(AudioManager::class.java).ringerMode == AudioManager.RINGER_MODE_NORMAL) {
                ringtone = runCatching { RingtoneManager.getRingtone(context, RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE))?.apply {
                    if (Build.VERSION.SDK_INT >= 28) isLooping = true
                    play()
                } }.getOrNull()
            }
        }
        if (call.phase != "incoming") { ringtone?.stop(); ringtone = null }
        IrisConnectionService.updateCurrent(call)
        try {
            ContextCompat.startForegroundService(context, Intent(context, IrisCallService::class.java))
        } catch (_: RuntimeException) {
            // Some devices reject a background start; the call stays answerable in the app.
            if (call.phase == "connected") fail("Audio unavailable")
        }
        if (serviceReady) syncMedia()
    }

    internal fun serviceStarted() { serviceReady = true; syncMedia() }
    internal fun serviceStopped() { serviceReady = false; stopMedia() }
    internal fun snapshot() = current

    private fun syncMedia() {
        val call = current ?: return
        if (call.phase != "connected" || failedCallId == call.callId) return
        val quality = CallQuality(app.preferences.value.callQuality, call.maxBitrateBps.toInt())
        if (mediaCallId != call.callId) {
            stopMedia()
            mediaCallId = call.callId
            if (!granted(Manifest.permission.RECORD_AUDIO)) { fail("Microphone unavailable"); return }
            try {
                audioRoute = CallAudioRoute(context) { if (isActive(call.callId)) fail("Audio unavailable") }
                audioRoute?.start(mutableSpeaker.value)
                audio = CallAudio(context,
                    send = { bytes, timestamp -> if (isActive(call.callId)) app.dispatch(AppAction.SendCallMedia(call.callId, 1u, timestamp, false, bytes)) },
                    failed = { if (isActive(call.callId)) fail("Audio unavailable") })
                audio?.start(call.muted)
                if (call.videoCapable) video = CallVideoMedia(context,
                    send = { bytes, timestamp, key -> if (isActive(call.callId)) app.dispatch(AppAction.SendCallMedia(call.callId, 2u, timestamp, key, bytes)) },
                    requestKey = { if (isActive(call.callId)) app.dispatch(AppAction.RequestCallKeyFrame(call.callId)) },
                    cameraFailed = { if (isActive(call.callId)) failCamera() },
                    decoded = {})
                markConnected(call.callId)
            } catch (_: Exception) { fail("Couldn’t connect call"); return }
        }
        audio?.muted = call.muted
        if (call.video && !granted(Manifest.permission.CAMERA)) failCamera()
        video?.update(call.video && granted(Manifest.permission.CAMERA), quality, call.targetBitrateBps.toInt(), call.keyFrameGeneration)
        mutableLocalVideo.value = video?.localVideo?.takeIf { call.video }
        mutableRemoteVideo.value = video?.remoteVideo?.takeIf { call.remoteVideo }
    }

    private fun receive(media: AppUpdate.CallMedia) {
        if (!isActive(media.callId)) return
        when (media.kind.toInt()) {
            1 -> audio?.receive(media.sequence, media.data)
            2 -> video?.receive(media.sequence, media.data, media.timestampUs, media.keyFrame)
        }
    }

    private fun markConnected(id: String) { scope.launch(Dispatchers.Main.immediate) {
        if (isActive(id) && !connectedSent) {
            connectedSent = true
            app.dispatch(AppAction.SetCallMediaConnected(id, true))
        }
    } }

    private fun isActive(id: String) = current?.let { it.callId == id && it.phase == "connected" && failedCallId != id && mediaCallId == id } == true

    private fun fail(message: String) { scope.launch(Dispatchers.Main.immediate) {
        failedCallId = current?.callId
        mutableError.value = message
        stopMedia()
        current?.takeIf { it.phase != "ended" }?.let { app.dispatch(AppAction.EndCall(it.callId)) }
    } }

    private fun failCamera() { scope.launch(Dispatchers.Main.immediate) {
        mutableLocalVideo.value = null
        mutableError.value = "Camera unavailable"
        if (current?.video == true) app.dispatch(AppAction.SetCallVideoEnabled(false))
    } }

    fun setSpeaker(enabled: Boolean) { mutableSpeaker.value = enabled; audioRoute?.setSpeaker(enabled) }
    fun requestAnswer(callId: String) { mutableAnswerRequest.value = callId }
    fun clearAnswerRequest() { mutableAnswerRequest.value = null }
    private fun granted(permission: String) = ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    private fun stopMedia() {
        audio?.close(); audio = null
        video?.close(); video = null
        connectedSent = false
        audioRoute?.close(); audioRoute = null
        mediaCallId = null
        mutableRemoteVideo.value = null
        mutableLocalVideo.value = null
    }
}
