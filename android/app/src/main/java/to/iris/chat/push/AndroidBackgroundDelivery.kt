package to.iris.chat.push

import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.content.ContextCompat
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import to.iris.chat.IrisDebugLog
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.core.AppManager
import to.iris.chat.rust.AppState
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.decidePendingNotifications
import to.iris.chat.rust.routerOpenChatId

/** Reuse the encrypted transport when this device has no working Google push provider. */
class AndroidBackgroundDelivery(
    private val context: Context,
    private val app: AppManager,
    scope: CoroutineScope,
) {
    private var requested = false
    private var previous: AppState? = null
    private var lastWanted: Boolean? = null
    private val googlePushAvailable = GoogleApiAvailabilityLight.getInstance()
        .isGooglePlayServicesAvailable(context) == ConnectionResult.SUCCESS

    init {
        app.setGooglePushAvailable(googlePushAvailable)
        scope.launch(Dispatchers.Main.immediate) {
            combine(app.state, app.bootstrapState, app.appForegrounded) { state, bootstrap, foreground ->
                Triple(state, bootstrap, foreground)
            }.collect { (state, bootstrap, foreground) ->
                val old = previous
                previous = state
                // These flows can arrive separately; never stop a sticky
                // receiver while the account snapshot itself is still loading.
                if (bootstrap is AccountBootstrapState.Loading || state.busy.restoringSession) return@collect
                val wanted = !googlePushAvailable &&
                    state.account?.authorizationState == DeviceAuthorizationState.AUTHORIZED &&
                    (state.preferences.desktopNotificationsEnabled || state.preferences.voiceCallsEnabled ||
                        state.preferences.videoCallsEnabled)
                if (lastWanted != wanted) {
                    IrisDebugLog.d("IrisPush", "background receiving=$wanted foreground=$foreground " +
                        "bootstrap=${bootstrap.javaClass.simpleName} authorization=${state.account?.authorizationState}")
                    lastWanted = wanted
                    context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).edit()
                        .putBoolean(ENABLED, wanted).apply()
                }
                if (!wanted) {
                    if (requested) context.stopService(Intent(context, BackgroundMessageService::class.java))
                    requested = false
                    app.setBackgroundReceiving(false)
                    return@collect
                }
                // Start while a user-visible activity allows it. START_STICKY and
                // the boot receiver cover process recreation without polling.
                if (!requested && foreground) {
                    try {
                        ContextCompat.startForegroundService(context, Intent(context, BackgroundMessageService::class.java))
                        requested = true
                        app.setBackgroundReceiving(true)
                    } catch (error: RuntimeException) {
                        Log.w("IrisPush", "Could not start background receiving", error)
                    }
                }
                if (old?.account?.publicKeyHex == state.account?.publicKeyHex && old?.account != null &&
                    old.chatList != state.chatList) {
                    val owner = state.account!!.publicKeyHex
                    decidePendingNotifications(old.chatList, state.chatList, state.preferences,
                        foreground, routerOpenChatId(state.router)).forEach { candidate ->
                        MobilePushNotifier.showLocal(context, candidate, owner)
                    }
                    MobilePushNotifier.dismissLocalRead(context, state.chatList, owner)
                }
            }
        }
    }

    fun serviceStarted() {
        IrisDebugLog.d("IrisPush", "background service started wanted=$lastWanted")
        if (lastWanted == false) {
            context.stopService(Intent(context, BackgroundMessageService::class.java))
            return
        }
        requested = true
        app.setBackgroundReceiving(true)
    }

    fun serviceStopped() {
        IrisDebugLog.d("IrisPush", "background service stopped")
        requested = false
        app.setBackgroundReceiving(false)
    }

    companion object {
        private const val PREFERENCES = "iris_background_receiving"
        private const val ENABLED = "enabled"
        fun shouldRestore(context: Context): Boolean = context
            .getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).getBoolean(ENABLED, false)
    }
}
