package to.iris.chat.calls

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioManager
import android.media.Ringtone
import android.media.RingtoneManager
import android.os.Build
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.CallSnapshot

class IrisCallRuntime(private val context: Context, private val app: AppManager, private val scope: CoroutineScope) {
    private val mutableRemoteFrame = MutableStateFlow<Bitmap?>(null)
    private val mutableLocalFrame = MutableStateFlow<Bitmap?>(null)
    private val mutableError = MutableStateFlow<String?>(null)
    private val mutableSpeaker = MutableStateFlow(false)
    private val mutableAnswerRequest = MutableStateFlow<String?>(null)
    val remoteFrame = mutableRemoteFrame.asStateFlow()
    val localFrame = mutableLocalFrame.asStateFlow()
    val error = mutableError.asStateFlow()
    val speaker = mutableSpeaker.asStateFlow()
    val answerRequest = mutableAnswerRequest.asStateFlow()
    private var ringtone: Ringtone? = null
    private val videoFrames = Channel<Pair<String, ByteArray>>(1, BufferOverflow.DROP_OLDEST)
    @Volatile private var current: CallSnapshot? = null
    @Volatile private var audio: CallAudio? = null
    private var camera: CallCamera? = null
    private var mediaCallId: String? = null
    private var announcedId: String? = null
    private var serviceReady = false

    init {
        app.setCallMediaReceiver(::receive)
        scope.launch(Dispatchers.Main.immediate) { app.call.collect { update(it) } }
        scope.launch(Dispatchers.Default) {
            for ((id, bytes) in videoFrames) {
                val call = current
                if (call?.callId == id && call.phase == "connected" && call.remoteVideo) {
                    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                    if (bounds.outWidth in 1..320 && bounds.outHeight in 1..320 &&
                        bounds.outWidth * bounds.outHeight <= 320 * 240) {
                        val frame = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                        if (current?.callId == id && current?.phase == "connected" && current?.remoteVideo == true) mutableRemoteFrame.value = frame
                    }
                }
            }
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
        if (!call.remoteVideo) mutableRemoteFrame.value = null
        try {
            ContextCompat.startForegroundService(context, Intent(context, IrisCallService::class.java))
        } catch (_: RuntimeException) {
            // Some devices reject a background start; the call stays answerable in the app.
            if (call.phase == "connected") failAudio()
        }
        if (serviceReady) syncMedia()
    }

    internal fun serviceStarted() { serviceReady = true; syncMedia() }
    internal fun serviceStopped() { serviceReady = false; stopMedia() }
    internal fun snapshot() = current

    private fun syncMedia() {
        val call = current ?: return
        if (call.phase != "connected") return
        if (mediaCallId != call.callId) {
            stopMedia()
            mediaCallId = call.callId
            if (!granted(Manifest.permission.RECORD_AUDIO)) { failAudio(); return }
            try {
                val capture = CallAudio(context, scope,
                    send = { bytes -> send(call.callId, 1u, bytes) }, failed = ::failAudio)
                audio = capture
                capture.muted = call.muted
                capture.start(mutableSpeaker.value)
            } catch (_: Exception) { failAudio(); return }
        }
        audio?.muted = call.muted
        if (call.video && camera == null) {
            if (!granted(Manifest.permission.CAMERA)) { failCamera(); return }
            try {
                val capture = CallCamera(context,
                    send = { bytes -> send(call.callId, 2u, bytes) },
                    preview = {
                        if (current?.callId == call.callId && current?.phase == "connected" && current?.video == true) mutableLocalFrame.value = it
                    }, failed = ::failCamera)
                camera = capture
                capture.start()
            } catch (_: Exception) { failCamera() }
        } else if (!call.video) {
            camera?.close(); camera = null; mutableLocalFrame.value = null
        }
    }

    private fun send(callId: String, kind: UByte, bytes: ByteArray) {
        if (current?.callId == callId && current?.phase == "connected") {
            app.dispatch(AppAction.SendCallMedia(callId, kind, bytes))
        }
    }

    private fun receive(callId: String, kind: UByte, bytes: ByteArray) {
        val call = current
        if (call?.callId != callId || call.phase != "connected") return
        when (kind.toInt()) {
            1 -> audio?.receive(bytes)
            2 -> if (bytes.size in 1..65_536 && call.remoteVideo) videoFrames.trySend(callId to bytes)
        }
    }

    private fun failAudio() { scope.launch(Dispatchers.Main.immediate) {
        mutableError.value = "Microphone unavailable"
        stopMedia()
        current?.takeIf { it.phase != "ended" }?.let { app.dispatch(AppAction.EndCall(it.callId)) }
    } }

    private fun failCamera() { scope.launch(Dispatchers.Main.immediate) {
        camera?.close(); camera = null; mutableLocalFrame.value = null
        mutableError.value = "Camera unavailable"
        if (current?.video == true) app.dispatch(AppAction.SetCallVideoEnabled(false))
    } }

    fun setSpeaker(enabled: Boolean) { mutableSpeaker.value = enabled; audio?.setSpeaker(enabled) }
    fun requestAnswer(callId: String) { mutableAnswerRequest.value = callId }
    fun clearAnswerRequest() { mutableAnswerRequest.value = null }
    private fun granted(permission: String) = ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    private fun stopMedia() {
        audio?.close(); audio = null
        camera?.close(); camera = null
        mediaCallId = null
        mutableRemoteFrame.value = null
        mutableLocalFrame.value = null
    }
}
