package to.iris.chat.calls

import android.content.Context
import android.os.Handler
import android.os.HandlerThread
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import org.webrtc.Camera1Enumerator
import org.webrtc.Camera2Enumerator
import org.webrtc.CameraEnumerator
import org.webrtc.CameraVideoCapturer
import org.webrtc.CapturerObserver
import org.webrtc.EglBase
import org.webrtc.PeerConnectionFactory
import org.webrtc.SurfaceTextureHelper
import org.webrtc.VideoFrame

/** Camera textures and rendering use native helpers; encoded media travels only through FIPS. */
internal class CallVideoMedia(
    private val context: Context,
    send: (ByteArray, ULong, Boolean) -> Unit,
    requestKey: () -> Unit,
    private val cameraFailed: () -> Unit,
    decoded: () -> Unit = {},
) : AutoCloseable {
    private val egl = EglBase.create()
    private val thread = HandlerThread("Iris-camera-control").apply { start() }
    private val handler = Handler(thread.looper)
    private val closed = AtomicBoolean(false)
    private val generation = AtomicLong()
    private val privacyLock = Any()
    val localVideo = CallVideoStream(egl.eglBaseContext, true)
    val remoteVideo = CallVideoStream(egl.eglBaseContext, false)
    private val encoder: CallVideoEncoder
    private val decoder: CallVideoDecoder
    private var camera: CameraVideoCapturer? = null
    private var textures: SurfaceTextureHelper? = null
    private var capturing = false
    private var high = false
    private var keyGeneration: UInt? = null
    @Volatile private var enabled = false

    init {
        initialize(context)
        encoder = CallVideoEncoder(egl.eglBaseContext, send, ::cameraError)
        decoder = CallVideoDecoder(egl.eglBaseContext, remoteVideo, requestKey, decoded)
    }

    fun update(enabled: Boolean, quality: CallQuality, targetBitrate: Int, keyGeneration: UInt) {
        val requestKey = this.keyGeneration != keyGeneration || enabled && !this.enabled
        this.keyGeneration = keyGeneration
        val ticket = synchronized(privacyLock) {
            if (this.enabled != enabled) generation.incrementAndGet()
            this.enabled = enabled
            encoder.update(enabled, quality.profile, minOf(quality.bitrate, targetBitrate), requestKey)
            generation.get()
        }
        handler.post {
            if (closed.get()) return@post
            try {
                val nextHigh = quality.profile == "high"
                if (enabled) {
                    if (!this.enabled || ticket != generation.get()) return@post
                    if (camera == null) {
                        val enumerator: CameraEnumerator = if (Camera2Enumerator.isSupported(context)) Camera2Enumerator(context)
                            else Camera1Enumerator(true)
                        val name = enumerator.deviceNames.firstOrNull { enumerator.isFrontFacing(it) }
                            ?: enumerator.deviceNames.first()
                        val capture = checkNotNull(enumerator.createCapturer(name, cameraEvents(ticket)))
                        camera = capture
                        val helper = SurfaceTextureHelper.create("Iris-camera", egl.eglBaseContext).also { textures = it }
                        capture.initialize(helper, context, observer(ticket))
                    }
                    val width = if (nextHigh) 1920 else 1280
                    val height = if (nextHigh) 1080 else 720
                    if (!capturing) { camera?.startCapture(width, height, 30); capturing = true }
                    else if (high != nextHigh) camera?.changeCaptureFormat(width, height, 30)
                    high = nextHigh
                } else stopCamera()
            } catch (_: Exception) { cameraError() }
        }
    }

    fun receive(sequence: UInt, data: ByteArray, timestamp: ULong, key: Boolean) = decoder.receive(sequence, data, timestamp, key)

    private fun cameraError() {
        if (!closed.get()) {
            synchronized(privacyLock) {
                enabled = false
                generation.incrementAndGet()
                encoder.update(false, "auto", 100_000, false)
            }
            handler.post { stopCamera() }
            cameraFailed()
        }
    }

    private fun observer(ticket: Long) = object : CapturerObserver {
        override fun onCapturerStarted(success: Boolean) { if (!success && ticket == generation.get()) cameraError() }
        override fun onCapturerStopped() = Unit
        override fun onFrameCaptured(frame: VideoFrame) {
            synchronized(privacyLock) {
                if (!closed.get() && enabled && ticket == generation.get()) { localVideo.onFrame(frame); encoder.frame(frame) }
            }
        }
    }
    private fun cameraEvents(ticket: Long) = object : CameraVideoCapturer.CameraEventsHandler {
        override fun onCameraError(error: String) = if (ticket == generation.get()) cameraError() else Unit
        override fun onCameraDisconnected() = if (ticket == generation.get()) cameraError() else Unit
        override fun onCameraFreezed(error: String) = if (ticket == generation.get()) cameraError() else Unit
        override fun onCameraOpening(name: String) = Unit
        override fun onFirstFrameAvailable() = Unit
        override fun onCameraClosed() = Unit
    }

    private fun stopCamera() {
        runCatching { if (capturing) camera?.stopCapture() }
        runCatching { camera?.dispose() }; camera = null; capturing = false
        runCatching { textures?.dispose() }; textures = null
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        enabled = false
        encoder.close(); decoder.close()
        localVideo.close(); remoteVideo.close()
        handler.post {
            stopCamera()
            encoder.awaitClosed(); decoder.awaitClosed()
            egl.release(); thread.quitSafely()
        }
    }

    companion object {
        private var initialized = false
        @Synchronized private fun initialize(context: Context) {
            if (!initialized) {
                // Loads only the camera/EGL JNI helpers. No peer connection or network engine is created.
                PeerConnectionFactory.initialize(PeerConnectionFactory.InitializationOptions.builder(context.applicationContext).createInitializationOptions())
                initialized = true
            }
        }
    }
}
