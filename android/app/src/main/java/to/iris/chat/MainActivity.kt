package to.iris.chat

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Parcelable
import android.util.Log
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.chat.core.AppContainer
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.OutgoingAttachment
import to.iris.chat.rust.isValidPeerInput
import to.iris.chat.rust.normalizePeerInput
import to.iris.chat.ui.navigation.NdrApp
import to.iris.chat.ui.screens.copyAttachmentToCache
import to.iris.chat.ui.theme.IrisChatTheme

class MainActivity : ComponentActivity() {
    private lateinit var container: AppContainer
    private var pendingNearbyPermissionAction: (() -> Unit)? = null
    private val nearbyPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
            val action = pendingNearbyPermissionAction
            pendingNearbyPermissionAction = null
            if (action != null && grants.isNotEmpty() && grants.values.all { it }) {
                action()
            } else if (action != null) {
                container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.d(TAG, "onCreate")
        container = (application as IrisChatApp).container
        handleLaunchIntent(intent)

        setContent {
            IrisChatTheme {
                NdrApp(
                    container = container,
                    onNearbyVisibilityChange = ::setNearbyVisible,
                )
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
        restoreNearbyVisibilityPreference()
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
        if (intent?.action == Intent.ACTION_SEND || intent?.action == Intent.ACTION_SEND_MULTIPLE) {
            handleShareIntent(intent)
            return
        }
        if (intent?.action == Intent.ACTION_VIEW) {
            handleChatLink(intent.data)
        }
    }

    private fun handleShareIntent(intent: Intent) {
        val text = shareTextFromIntent(intent)
        val streamUris = streamUrisFromIntent(intent)
        if (text.isBlank() && streamUris.isEmpty()) {
            return
        }
        lifecycleScope.launch {
            val attachments =
                withContext(Dispatchers.IO) {
                    streamUris.mapNotNull { uri ->
                        copyAttachmentToCache(this@MainActivity, uri)?.let { attachment ->
                            OutgoingAttachment(
                                filePath = attachment.path,
                                filename = attachment.filename,
                            )
                        }
                    }
                }
            container.appManager.receiveShare(text, attachments)
        }
    }

    private fun shareTextFromIntent(intent: Intent): String {
        val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty()
        if (text.isNotBlank()) {
            return text
        }
        return intent.getCharSequenceExtra(Intent.EXTRA_SUBJECT)?.toString().orEmpty()
    }

    private fun streamUrisFromIntent(intent: Intent): List<Uri> {
        val uris = mutableListOf<Uri>()
        if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
            uris += intent.parcelableArrayListExtraCompat<Uri>(Intent.EXTRA_STREAM).orEmpty()
        } else {
            intent.parcelableExtraCompat<Uri>(Intent.EXTRA_STREAM)?.let { uris += it }
        }
        intent.clipData?.let { clipData ->
            for (index in 0 until clipData.itemCount) {
                clipData.getItemAt(index).uri?.let { uris += it }
            }
        }
        return uris.distinctBy { it.toString() }
    }

    private inline fun <reified T : Parcelable> Intent.parcelableExtraCompat(name: String): T? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableExtra(name, T::class.java)
        } else {
            @Suppress("DEPRECATION")
            getParcelableExtra(name) as? T
        }

    private inline fun <reified T : Parcelable> Intent.parcelableArrayListExtraCompat(name: String): ArrayList<T>? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableArrayListExtra(name, T::class.java)
        } else {
            @Suppress("DEPRECATION")
            getParcelableArrayListExtra(name)
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

    private fun setNearbyVisible(visible: Boolean) {
        if (!visible) {
            container.nearbyIrisService.setVisible(false)
            container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
            return
        }
        requestNearbyPermissionIfNeeded {
            container.nearbyIrisService.setVisible(true)
            container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(true))
        }
    }

    private fun nearbyPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_CONNECT,
                Manifest.permission.BLUETOOTH_ADVERTISE,
            )
        } else {
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    private fun requestNearbyPermissionIfNeeded(onGranted: () -> Unit) {
        val permissions = nearbyPermissions()
        val missing =
            permissions.filter {
                ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
            }
        if (missing.isEmpty()) {
            onGranted()
            return
        }
        if (pendingNearbyPermissionAction != null) {
            pendingNearbyPermissionAction = onGranted
            return
        }
        pendingNearbyPermissionAction = onGranted
        nearbyPermissionLauncher.launch(missing.toTypedArray())
    }

    private fun restoreNearbyVisibilityPreference() {
        if (!container.appManager.state.value.preferences.nearbyBluetoothEnabled) {
            return
        }
        if (!container.nearbyIrisService.hasBluetoothPermission()) {
            return
        }
        container.nearbyIrisService.setVisible(true)
    }
}
