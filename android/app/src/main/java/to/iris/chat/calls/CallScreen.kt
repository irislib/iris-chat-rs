package to.iris.chat.calls

import android.Manifest
import android.content.pm.PackageManager
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import to.iris.chat.core.AppContainer
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.CallSnapshot

@Composable
private fun rememberCallPermissionAction(): (Boolean, () -> Unit) -> Unit {
    val context = LocalContext.current
    var pending by remember { mutableStateOf<(() -> Unit)?>(null) }
    var requested by remember { mutableStateOf(emptyArray<String>()) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val action = pending
        pending = null
        if (requested.all { result[it] == true || ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }) action?.invoke()
        else Toast.makeText(context, "Allow microphone${if (Manifest.permission.CAMERA in requested) " and camera" else ""} access to call", Toast.LENGTH_LONG).show()
    }
    return { video, action ->
        val permissions = if (video) arrayOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.CAMERA)
            else arrayOf(Manifest.permission.RECORD_AUDIO)
        if (permissions.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }) action()
        else { pending = action; requested = permissions; launcher.launch(permissions) }
    }
}

@Composable
fun ChatCallButtons(app: AppManager, chatId: String) {
    val preferences by app.preferences.collectAsStateWithLifecycle()
    val call by app.call.collectAsStateWithLifecycle()
    val permissions = rememberCallPermissionAction()
    val available = call == null || call?.phase == "ended"
    if (preferences.voiceCallsEnabled) IconButton(
        onClick = { permissions(false) { app.dispatch(AppAction.StartCall(chatId, false)) } },
        enabled = available, modifier = Modifier.testTag("chatVoiceCallButton"),
    ) { Icon(Icons.Filled.Call, "Voice call") }
    if (preferences.videoCallsEnabled) IconButton(
        onClick = { permissions(true) { app.dispatch(AppAction.StartCall(chatId, true)) } },
        enabled = available, modifier = Modifier.testTag("chatVideoCallButton"),
    ) { Icon(Icons.Filled.Videocam, "Video call") }
}

@Composable
fun CallOverlay(container: AppContainer) {
    val app = container.appManager
    val call by app.call.collectAsStateWithLifecycle()
    val preferences by app.preferences.collectAsStateWithLifecycle()
    val remote by container.callRuntime.remoteVideo.collectAsStateWithLifecycle()
    val local by container.callRuntime.localVideo.collectAsStateWithLifecycle()
    val error by container.callRuntime.error.collectAsStateWithLifecycle()
    val speaker by container.callRuntime.speaker.collectAsStateWithLifecycle()
    val answerRequest by container.callRuntime.answerRequest.collectAsStateWithLifecycle()
    var dismissedId by remember { mutableStateOf<String?>(null) }
    val active = call ?: return
    if (active.phase == "ended" && dismissedId == active.callId) return
    val permissions = rememberCallPermissionAction()
    LaunchedEffect(answerRequest, active.callId, active.phase) {
        if (answerRequest == active.callId && active.phase == "incoming") {
            container.callRuntime.clearAnswerRequest()
            permissions(active.video) { app.dispatch(AppAction.AnswerCall(active.callId)) }
        }
    }
    var now by remember { mutableStateOf(System.currentTimeMillis() / 1_000L) }
    LaunchedEffect(active.callId, active.phase) {
        while (active.phase == "connected") { now = System.currentTimeMillis() / 1_000L; delay(1_000L) }
    }
    CallSurface(active, preferences.voiceCallsEnabled, preferences.videoCallsEnabled, remote, local, error,
        speaker, now, permissions, app::dispatch, container.callRuntime::setSpeaker) { dismissedId = active.callId }
}

@Composable
internal fun CallSurface(
    active: CallSnapshot,
    voiceAllowed: Boolean,
    videoAllowed: Boolean,
    remote: CallVideoStream?,
    local: CallVideoStream?,
    error: String?,
    speaker: Boolean,
    now: Long,
    permissions: (Boolean, () -> Unit) -> Unit,
    onAction: (AppAction) -> Unit,
    onSpeaker: (Boolean) -> Unit,
    onDismiss: () -> Unit,
) {
    Dialog(onDismissRequest = { if (active.phase == "ended") onDismiss() },
        properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnBackPress = false, decorFitsSystemWindows = false)) {
        Box(Modifier.fillMaxSize().background(Color(0xFF14201F)).testTag("callScreen")) {
            remote?.let {
                CallVideo(it, Modifier.fillMaxSize())
                Box(Modifier.fillMaxSize().background(Brush.verticalGradient(
                    0f to Color.Black.copy(alpha = 0.65f), 0.3f to Color.Transparent,
                    0.55f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.75f),
                )))
            }
            Column(Modifier.fillMaxSize().systemBarsPadding().padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Spacer(Modifier.height(36.dp))
                Text(active.peerName, color = Color.White, style = MaterialTheme.typography.headlineLarge, textAlign = TextAlign.Center)
                Spacer(Modifier.height(12.dp))
                val elapsed = (now - (active.connectedAtSecs?.toLong() ?: now)).coerceAtLeast(0)
                Text(when (active.phase) {
                    "incoming" -> if (active.video) "Incoming video call" else "Incoming voice call"
                    "connected" -> if (active.mediaConnected) "%d:%02d".format(elapsed / 60, elapsed % 60) else "Connecting…"
                    "ended" -> error ?: active.endReason ?: "Call ended"
                    else -> "Calling…"
                }, color = Color.White.copy(alpha = 0.8f), style = MaterialTheme.typography.titleMedium)
                if (active.remoteMuted && active.phase == "connected") Text("Microphone muted", color = Color.White.copy(alpha = 0.7f))
                Spacer(Modifier.weight(1f))
                local?.let { CallVideo(it, Modifier.align(Alignment.End).size(112.dp, 150.dp).clip(MaterialTheme.shapes.large), overlay = true) }
                Spacer(Modifier.height(24.dp))
                if (active.phase == "incoming") {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
                        CallControl("Decline", Icons.Filled.CallEnd, Color(0xFFE34B52)) { onAction(AppAction.EndCall(active.callId)) }
                        CallControl("Answer", if (active.video) Icons.Filled.Videocam else Icons.Filled.Call, Color(0xFF268F68)) {
                            permissions(active.video) { onAction(AppAction.AnswerCall(active.callId)) }
                        }
                    }
                    if (active.video && voiceAllowed) TextButton(onClick = {
                        permissions(false) { onAction(AppAction.AnswerCallWithVoice(active.callId)) }
                    }) { Text("Answer with voice", color = Color.White) }
                } else if (active.phase == "ended") {
                    Button(onClick = { onDismiss() }) { Text("Done") }
                } else {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
                        CallControl(if (active.muted) "Unmute" else "Mute", if (active.muted) Icons.Filled.MicOff else Icons.Filled.Mic,
                            if (active.muted) Color(0xFF576B69) else Color(0xFF30413F)) { onAction(AppAction.SetCallMuted(!active.muted)) }
                        CallControl("Speaker", Icons.Filled.VolumeUp, if (speaker) Color(0xFF576B69) else Color(0xFF30413F)) { onSpeaker(!speaker) }
                        if (active.videoCapable && videoAllowed) CallControl("Camera", if (active.video) Icons.Filled.Videocam else Icons.Filled.VideocamOff) {
                            if (active.video) onAction(AppAction.SetCallVideoEnabled(false))
                            else permissions(true) { onAction(AppAction.SetCallVideoEnabled(true)) }
                        }
                    }
                    Spacer(Modifier.height(24.dp))
                    CallControl("End call", Icons.Filled.CallEnd, Color(0xFFE34B52)) { onAction(AppAction.EndCall(active.callId)) }
                }
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}

@Composable
private fun CallControl(label: String, icon: ImageVector, color: Color = Color(0xFF30413F), onClick: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        IconButton(onClick = onClick, modifier = Modifier.size(64.dp).clip(CircleShape).background(color).testTag("call$label")) {
            Icon(icon, label, Modifier.size(28.dp), tint = Color.White)
        }
        Spacer(Modifier.height(8.dp))
        Text(label, color = Color.White, style = MaterialTheme.typography.labelMedium)
    }
}
