package to.iris.chat.calls

import android.annotation.SuppressLint
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.telecom.Connection
import android.telecom.CallAudioState
import android.telecom.ConnectionRequest
import android.telecom.ConnectionService
import android.telecom.DisconnectCause
import android.telecom.PhoneAccount
import android.telecom.PhoneAccountHandle
import android.telecom.TelecomManager
import android.telecom.VideoProfile
import to.iris.chat.IrisChatApp
import to.iris.chat.MainActivity
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.CallSnapshot

/** Self-managed calls integrate with headset controls and the system's other calls. */
class IrisConnectionService : ConnectionService() {
    override fun onConnectionServiceFocusGained() {
        (application as IrisChatApp).container.callRuntime.telecomFocusChanged(true)
    }
    override fun onConnectionServiceFocusLost() {
        (application as IrisChatApp).container.callRuntime.telecomFocusChanged(false)
        connectionServiceFocusReleased()
    }
    override fun onCreateIncomingConnection(account: PhoneAccountHandle?, request: ConnectionRequest?): Connection = create(true)
    override fun onCreateOutgoingConnection(account: PhoneAccountHandle?, request: ConnectionRequest?): Connection = create(false)
    override fun onCreateIncomingConnectionFailed(account: PhoneAccountHandle?, request: ConnectionRequest?) { endRejectedCall() }
    override fun onCreateOutgoingConnectionFailed(account: PhoneAccountHandle?, request: ConnectionRequest?) { endRejectedCall() }

    private fun endRejectedCall() {
        val app = (application as IrisChatApp).container.appManager
        app.call.value?.takeIf { it.phase != "ended" }?.let { app.dispatch(AppAction.EndCall(it.callId)) }
    }

    private fun create(incoming: Boolean): Connection {
        val app = (application as IrisChatApp).container.appManager
        val call = app.call.value?.takeIf { it.phase != "ended" }
            ?: return Connection.createFailedConnection(DisconnectCause(DisconnectCause.CANCELED))
        return object : Connection() {
            override fun onAnswer() { openCall() }
            override fun onAnswer(videoState: Int) { openCall() }
            override fun onReject() { end() }
            override fun onDisconnect() { end() }
            override fun onAbort() { end() }
            // Always enter the activity before acquiring microphone/camera while-in-use permissions.
            private fun openCall() { startActivity(Intent(this@IrisConnectionService, MainActivity::class.java)
                .setAction("to.iris.chat.ANSWER_CALL").putExtra("callId", call.callId)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)) }
            private fun end() { app.dispatch(AppAction.EndCall(call.callId)) }
        }.apply {
            connectionProperties = Connection.PROPERTY_SELF_MANAGED
            connectionCapabilities = Connection.CAPABILITY_MUTE
            audioModeIsVoip = true
            setCallerDisplayName(call.peerName, TelecomManager.PRESENTATION_ALLOWED)
            setAddress(Uri.fromParts("sip", call.callId, null), TelecomManager.PRESENTATION_ALLOWED)
            if (incoming) setRinging() else setDialing()
            current = this
            currentId = call.callId
            updateCurrent(call)
        }
    }

    companion object {
        private var current: Connection? = null
        private var currentId: String? = null

        @Suppress("DEPRECATION")
        fun setSpeaker(enabled: Boolean) {
            current?.setAudioRoute(if (enabled) CallAudioState.ROUTE_SPEAKER else CallAudioState.ROUTE_WIRED_OR_EARPIECE)
        }

        @SuppressLint("MissingPermission")
        fun announce(context: Context, call: CallSnapshot) {
            val telecom = context.getSystemService(TelecomManager::class.java) ?: return
            val handle = PhoneAccountHandle(ComponentName(context, IrisConnectionService::class.java), "iris-calls")
            runCatching {
                telecom.registerPhoneAccount(PhoneAccount.builder(handle, "Iris")
                    .setCapabilities(PhoneAccount.CAPABILITY_SELF_MANAGED)
                    .setSupportedUriSchemes(listOf(PhoneAccount.SCHEME_SIP)).build())
                if (call.phase == "incoming") telecom.addNewIncomingCall(handle, Bundle())
                else telecom.placeCall(Uri.fromParts("sip", call.callId, null), Bundle().apply {
                    putParcelable(TelecomManager.EXTRA_PHONE_ACCOUNT_HANDLE, handle)
                })
            }
        }

        fun updateCurrent(call: CallSnapshot) {
            if (currentId != call.callId) return
            current?.apply {
                videoState = if (call.video) VideoProfile.STATE_BIDIRECTIONAL else VideoProfile.STATE_AUDIO_ONLY
                if (call.phase == "connected") setActive()
            }
        }

        fun finishCurrent() {
            current?.apply { setDisconnected(DisconnectCause(DisconnectCause.LOCAL)); destroy() }
            current = null
            currentId = null
        }
    }
}
