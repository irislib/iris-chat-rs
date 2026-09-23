package to.iris.chat.ui.screens

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import to.iris.chat.core.AppManager
import to.iris.chat.ui.components.IrisPrimaryButton
import to.iris.chat.ui.components.IrisSecondaryButton
import to.iris.chat.ui.components.rememberIrisClipboard
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.RemoteSignerPhase

@Composable
fun RemoteSignerScreen(appManager: AppManager, appState: AppState) {
    val clipboard = rememberIrisClipboard()
    val context = LocalContext.current
    val localBusy by appManager.signer.busy.collectAsStateWithLifecycle()
    var showingLinkInput by remember { mutableStateOf(false) }
    var signerLink by remember { mutableStateOf("") }
    val login = appState.remoteSignerLogin
    val awaitingApproval = login?.phase == RemoteSignerPhase.WAITING_FOR_APPROVAL ||
        login?.phase == RemoteSignerPhase.FINISHING
    val code = login?.connectionUri
    val bitmap = remember(code) { code?.let { createQrBitmap(it, 720) } }

    OnboardingScaffold(
        title = "Signer app/device",
        onBack = {
            appManager.signer.cancel()
            appManager.navigateBack()
        },
        bottomContent = {},
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().testTag("remoteSignerScreen"),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            IrisSecondaryButton(
                text = "Open signer app",
                enabled = !localBusy && !awaitingApproval,
                onClick = {
                    appManager.dispatch(AppAction.CancelRemoteSignerLogin)
                    appManager.signer.startLogin()
                },
                modifier = Modifier.fillMaxWidth().testTag("remoteSignerOpenAppAction"),
            )
            if (localBusy) {
                CircularProgressIndicator()
                Text("Approve in your signer app.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else if (bitmap != null && !awaitingApproval) {
                Text("Scan with your signer app.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                IrisQrCodeImage(bitmap, "Signer connection code", size = 260.dp, tag = "remoteSignerCode")
                IrisSecondaryButton(
                    text = "Copy code",
                    onClick = { code?.let { clipboard.setText("Signer code", it) } },
                    modifier = Modifier.fillMaxWidth(),
                )
            } else if (login != null) {
                CircularProgressIndicator()
                Text(
                    when (login.phase) {
                        RemoteSignerPhase.CONNECTING -> "Connecting…"
                        RemoteSignerPhase.WAITING_FOR_SIGNER -> "Waiting for your signer…"
                        RemoteSignerPhase.WAITING_FOR_APPROVAL -> "Approve in your signer app."
                        RemoteSignerPhase.FINISHING -> "Signing in…"
                    },
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                IrisSecondaryButton(
                    text = "Try again",
                    onClick = { appManager.dispatch(AppAction.StartRemoteSignerLogin) },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            login?.authUrl?.let { value ->
                val uri = Uri.parse(value)
                if (uri.scheme?.lowercase() in listOf("https", "http") && !uri.host.isNullOrBlank()) {
                    IrisPrimaryButton(
                        text = "Open approval",
                        onClick = { runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, uri)) } },
                        modifier = Modifier.fillMaxWidth().testTag("remoteSignerApprovalAction"),
                    )
                }
            }
            if (showingLinkInput && !localBusy) {
                TextField(
                    value = signerLink,
                    onValueChange = { signerLink = it },
                    placeholder = { Text("Signer link") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().testTag("remoteSignerLinkInput"),
                )
                IrisPrimaryButton(
                    text = "Connect",
                    enabled = signerLink.isNotBlank(),
                    onClick = {
                        appManager.dispatch(AppAction.ConnectRemoteSigner(signerLink))
                        showingLinkInput = false
                        signerLink = ""
                    },
                    modifier = Modifier.fillMaxWidth().testTag("remoteSignerConnectAction"),
                )
            } else if (!localBusy && !awaitingApproval) {
                IrisSecondaryButton(
                    text = "Paste signer link",
                    onClick = {
                        clipboard.getText { signerLink = it }
                        showingLinkInput = true
                    },
                    modifier = Modifier.fillMaxWidth().testTag("remoteSignerPasteLink"),
                )
            }
            OnboardingMessageCard(message = appState.toast)
        }
    }
}
