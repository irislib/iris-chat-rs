package to.iris.chat

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color as AndroidColor
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Parcelable
import android.provider.Settings
import android.util.Log
import androidx.activity.SystemBarStyle
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.SideEffect
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
    private var pendingPermissionRequest: RuntimePermissionRequest? = null
    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
            val request = pendingPermissionRequest
            pendingPermissionRequest = null
            if (::container.isInitialized) {
                container.nearbyIrisService.refreshPermissionState()
            }
            if (request != null) {
                request.preferenceKeys.forEach(::markPermissionRequested)
                if (request.permissions.all { permission ->
                        grants[permission] == true ||
                            ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED
                    }
                ) {
                    request.onGranted()
                } else {
                    request.onDenied()
                    if (shouldOpenSettingsForPermissions(request.permissions, request.preferenceKeys)) {
                        openAppSettings()
                    }
                }
            }
        }

    private data class RuntimePermissionRequest(
        val permissions: List<String>,
        val preferenceKeys: List<String>,
        val onGranted: () -> Unit,
        val onDenied: () -> Unit,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        IrisDebugLog.d(TAG, "onCreate")
        container = (application as IrisChatApp).container
        handleLaunchIntent(intent)

        setContent {
            val darkTheme = isSystemInDarkTheme()
            SideEffect {
                syncSystemBars(darkTheme)
            }
            IrisChatTheme(darkTheme = darkTheme) {
                NdrApp(
                    container = container,
                    onNearbyVisibilityChange = ::setNearbyVisible,
                    onNearbyLanVisibilityChange = ::setNearbyLanVisible,
                )
            }
        }
    }

    private fun syncSystemBars(darkTheme: Boolean) {
        val lightBackground = AndroidColor.rgb(251, 252, 255)
        val systemBarStyle =
            if (darkTheme) {
                SystemBarStyle.dark(AndroidColor.BLACK)
            } else {
                SystemBarStyle.light(lightBackground, lightBackground)
            }
        enableEdgeToEdge(
            statusBarStyle = systemBarStyle,
            navigationBarStyle = systemBarStyle,
        )
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleLaunchIntent(intent)
    }

    override fun onUserInteraction() {
        super.onUserInteraction()
        if (::container.isInitialized) {
            container.appManager.recordUserActivity()
        }
    }

    override fun onStart() {
        super.onStart()
        IrisDebugLog.d(TAG, "onStart")
        requestNotificationPermissionIfNeeded()
        container.nearbyIrisService.refreshPermissionState()
        container.appManager.appForegrounded()
        restoreNearbyVisibilityPreference()
    }

    override fun onStop() {
        IrisDebugLog.d(TAG, "onStop")
        container.appManager.appBackgrounded()
        super.onStop()
    }

    private companion object {
        const val TAG = "NdrDebug"
        const val ACTION_OPEN_CHAT_LIST = "to.iris.chat.OPEN_CHAT_LIST"
        const val NOTIFICATION_PERMISSION_REQUEST = 1001
        const val PERMISSION_PREFS = "iris_runtime_permissions"
        const val BLUETOOTH_PERMISSION_KEY = "nearby_bluetooth"
        const val LOCAL_NETWORK_PERMISSION_KEY = "nearby_local_network"
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

        val decodedFragment = Uri.decode(uri.fragment.orEmpty())
        if (
            decodedFragment.contains("\"ephemeralKey\"") &&
            decodedFragment.contains("\"sharedSecret\"")
        ) {
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
        requestRuntimePermissionsIfNeeded(
            permissions = nearbyPermissions().toList(),
            preferenceKeys = listOf(BLUETOOTH_PERMISSION_KEY),
            onGranted = {
                container.nearbyIrisService.setVisible(true)
                container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(true))
            },
            onDenied = {
                container.nearbyIrisService.setVisible(false)
                container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
            },
        )
    }

    private fun setNearbyLanVisible(visible: Boolean) {
        if (!visible) {
            container.nearbyIrisService.setLocalNetworkVisible(false)
            container.appManager.dispatch(AppAction.SetNearbyLanEnabled(false))
            return
        }
        requestRuntimePermissionsIfNeeded(
            permissions = localNetworkPermissions().toList(),
            preferenceKeys = listOf(LOCAL_NETWORK_PERMISSION_KEY),
            onGranted = {
                container.nearbyIrisService.setLocalNetworkVisible(true)
                container.appManager.dispatch(AppAction.SetNearbyLanEnabled(true))
            },
            onDenied = {
                container.nearbyIrisService.setLocalNetworkVisible(false)
                container.appManager.dispatch(AppAction.SetNearbyLanEnabled(false))
            },
        )
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

    private fun localNetworkPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(Manifest.permission.NEARBY_WIFI_DEVICES)
        } else {
            emptyArray()
        }

    private fun requestRuntimePermissionsIfNeeded(
        permissions: List<String>,
        preferenceKeys: List<String>,
        onGranted: () -> Unit,
        onDenied: () -> Unit,
    ) {
        val missing =
            permissions.filter {
                ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
            }
        container.nearbyIrisService.refreshPermissionState()
        if (missing.isEmpty()) {
            onGranted()
            return
        }
        if (shouldOpenSettingsForPermissions(missing, preferenceKeys)) {
            onDenied()
            openAppSettings()
            return
        }
        if (pendingPermissionRequest != null) {
            return
        }
        pendingPermissionRequest = RuntimePermissionRequest(missing, preferenceKeys, onGranted, onDenied)
        permissionLauncher.launch(missing.toTypedArray())
    }

    private fun shouldOpenSettingsForPermissions(
        permissions: List<String>,
        preferenceKeys: List<String> = emptyList(),
    ): Boolean {
        val wasRequested = preferenceKeys.isNotEmpty() && preferenceKeys.all(::permissionWasRequested)
        return wasRequested && permissions.any { !ActivityCompat.shouldShowRequestPermissionRationale(this, it) }
    }

    private fun permissionWasRequested(preferenceKey: String): Boolean =
        getSharedPreferences(PERMISSION_PREFS, Context.MODE_PRIVATE).getBoolean(preferenceKey, false)

    private fun markPermissionRequested(preferenceKey: String) {
        getSharedPreferences(PERMISSION_PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(preferenceKey, true)
            .apply()
    }

    private fun openAppSettings() {
        val uri = Uri.fromParts("package", packageName, null)
        startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, uri))
    }

    private fun restoreNearbyVisibilityPreference() {
        val preferences = container.appManager.state.value.preferences
        if (!preferences.nearbyEnabled) {
            container.nearbyIrisService.setVisible(false)
            container.nearbyIrisService.setLocalNetworkVisible(false)
            return
        }
        if (preferences.nearbyBluetoothEnabled) {
            if (container.nearbyIrisService.hasBluetoothPermission()) {
                container.nearbyIrisService.setVisible(true)
            } else {
                container.appManager.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
            }
        }
        if (preferences.nearbyLanEnabled) {
            if (container.nearbyIrisService.hasLocalNetworkPermission()) {
                container.nearbyIrisService.setLocalNetworkVisible(true)
            } else {
                container.appManager.dispatch(AppAction.SetNearbyLanEnabled(false))
            }
        }
    }
}
