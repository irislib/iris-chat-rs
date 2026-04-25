package social.innode.ndr.demo

import android.Manifest
import android.content.Intent
import android.os.Bundle
import android.os.Build
import android.content.pm.PackageManager
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import social.innode.ndr.demo.core.AppContainer
import social.innode.ndr.demo.rust.AppAction
import social.innode.ndr.demo.ui.navigation.NdrApp
import social.innode.ndr.demo.ui.theme.IrisChatTheme

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
        super.onStop()
    }

    private companion object {
        const val TAG = "NdrDebug"
        const val ACTION_OPEN_CHAT_LIST = "social.innode.ndr.demo.OPEN_CHAT_LIST"
        const val NOTIFICATION_PERMISSION_REQUEST = 1001
    }

    private fun handleLaunchIntent(intent: Intent?) {
        if (intent?.action == ACTION_OPEN_CHAT_LIST) {
            container.appManager.dispatch(AppAction.UpdateScreenStack(emptyList()))
        }
    }

    private fun requestNotificationPermissionIfNeeded() {
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
