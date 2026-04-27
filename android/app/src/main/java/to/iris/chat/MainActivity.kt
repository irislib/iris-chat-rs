package to.iris.chat

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Build
import android.content.pm.PackageManager
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import to.iris.chat.core.AppContainer
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.navigation.NdrApp
import to.iris.chat.ui.theme.IrisChatTheme

class MainActivity : ComponentActivity() {
    private lateinit var container: AppContainer

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.d(TAG, "onCreate")
        container = (application as IrisChatApp).container
        handleLaunchIntent(intent)

        setContent {
            IrisChatTheme {
                NdrApp(container = container)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleLaunchIntent(intent)
    }

    override fun onStart() {
        super.onStart()
        Log.d(TAG, "onStart")
        requestNotificationPermissionIfNeeded()
        container.appManager.appForegrounded()
    }

    override fun onStop() {
        Log.d(TAG, "onStop")
        container.appManager.appBackgrounded()
        super.onStop()
    }

    private companion object {
        const val TAG = "NdrDebug"
        const val ACTION_OPEN_CHAT_LIST = "to.iris.chat.OPEN_CHAT_LIST"
        const val NOTIFICATION_PERMISSION_REQUEST = 1001
    }

    private fun handleLaunchIntent(intent: Intent?) {
        if (intent?.action == ACTION_OPEN_CHAT_LIST) {
            container.appManager.dispatch(AppAction.UpdateScreenStack(emptyList()))
            return
        }
        if (intent?.action == Intent.ACTION_VIEW) {
            handleChatLink(intent.data)
        }
    }

    private fun handleChatLink(uri: Uri?) {
        val raw = uri?.toString()?.trim().orEmpty()
        if (raw.isEmpty()) {
            return
        }
        val host = uri?.host?.lowercase()
        val scheme = uri?.scheme?.lowercase()
        if (scheme != "https" || host != "chat.iris.to") {
            return
        }

        val inviteInput = inviteInputFromChatLink(uri)
        if (inviteInput != null) {
            container.appManager.dispatch(AppAction.AcceptInvite(inviteInput))
            return
        }

        val peerInput = peerInputFromChatLink(uri)
        if (peerInput != null) {
            container.appManager.createChat(peerInput)
        }
    }

    private fun inviteInputFromChatLink(uri: Uri): String? {
        val pathSegments = uri.pathSegments
        if (pathSegments.firstOrNull()?.lowercase() == "invite" && pathSegments.size >= 2) {
            return uri.toString()
        }

        val fragmentSegments = uri.fragmentSegments()
        if (fragmentSegments.firstOrNull()?.lowercase() == "invite" && fragmentSegments.size >= 2) {
            return uri.toString()
        }

        return null
    }

    private fun peerInputFromChatLink(uri: Uri): String? {
        val candidates =
            listOfNotNull(
                uri.lastPathSegment,
                uri.fragmentSegments().firstOrNull(),
                uri.fragment,
            )

        for (candidate in candidates) {
            val normalized = normalizePeerInput(candidate)
            if (normalized.isNotBlank() && isValidPeerInput(normalized)) {
                return normalized
            }
        }

        return null
    }

    private fun Uri.fragmentSegments(): List<String> =
        fragment
            ?.trim()
            ?.removePrefix("/")
            ?.split("/")
            ?.filter(String::isNotBlank)
            .orEmpty()

    private fun requestNotificationPermissionIfNeeded() {
        if (BuildConfig.DEBUG) {
            return
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return
        }
        if (!container.appManager.state.value.preferences.desktopNotificationsEnabled) {
            return
        }
        if (
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        ActivityCompat.requestPermissions(
            this,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            NOTIFICATION_PERMISSION_REQUEST,
        )
    }
}
