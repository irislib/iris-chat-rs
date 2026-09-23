package to.iris.chat.calls

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import java.util.concurrent.CopyOnWriteArrayList
import org.webrtc.EglBase
import org.webrtc.RendererCommon
import org.webrtc.SurfaceViewRenderer
import org.webrtc.VideoFrame
import org.webrtc.VideoSink

/** Decoded and captured frames share the same native surface renderer. */
class CallVideoStream internal constructor(internal val egl: EglBase.Context, internal val mirror: Boolean) : VideoSink {
    private val sinks = CopyOnWriteArrayList<VideoSink>()
    internal fun add(sink: VideoSink) { sinks.add(sink) }
    internal fun remove(sink: VideoSink) { sinks.remove(sink) }
    internal fun close() { sinks.clear() }
    override fun onFrame(frame: VideoFrame) { sinks.forEach { it.onFrame(frame) } }
}

@Composable
internal fun CallVideo(stream: CallVideoStream, modifier: Modifier = Modifier, overlay: Boolean = false) {
    val context = LocalContext.current
    val renderer = remember(stream) {
        SurfaceViewRenderer(context).apply {
            init(stream.egl, null)
            setMirror(stream.mirror)
            setEnableHardwareScaler(true)
            setScalingType(RendererCommon.ScalingType.SCALE_ASPECT_FIT)
            setZOrderMediaOverlay(overlay)
        }
    }
    DisposableEffect(stream, renderer) {
        stream.add(renderer)
        onDispose { stream.remove(renderer); renderer.release() }
    }
    AndroidView(factory = { renderer }, modifier = modifier)
}
