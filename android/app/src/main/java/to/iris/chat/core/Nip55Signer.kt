package to.iris.chat.core

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONObject
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppState
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.peerInputToHex

data class SignerActivityRequest(
    val id: String,
    val type: String,
    val payload: String = "",
    val packageName: String? = null,
    val ownerPubkeyHex: String? = null,
) {
    fun intent(): Intent =
        Intent(Intent.ACTION_VIEW, Uri.parse("nostrsigner:$payload")).apply {
            putExtra("type", type)
            putExtra("id", id)
            packageName?.let { setPackage(it) }
            ownerPubkeyHex?.let { putExtra("current_user", it) }
        }
}

/**
 * Holds a single user-initiated signer exchange across Activity recreation. Nothing here is
 * persisted: after process death the user can safely retry with a fresh device authorization.
 */
class Nip55Signer(
    private val context: Context,
    private val scope: CoroutineScope,
    private val dispatch: (AppAction) -> Boolean,
    private val showError: (String) -> Unit,
) {
    private val mutableRequest = MutableStateFlow<SignerActivityRequest?>(null)
    val pendingRequest: StateFlow<SignerActivityRequest?> = mutableRequest.asStateFlow()
    private val mutableBusy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = mutableBusy.asStateFlow()
    private var launchedRequestId: String? = null
    private var signerPackages = emptySet<String>()
    private var selectedPackage: String? = null
    private var selectedPubkey: String? = null
    private var coreRequestId: String? = null
    private var waitingForCore = false
    private var observedCoreBusy = false
    private var timeout: Job? = null

    @Synchronized
    fun startLogin() {
        if (mutableBusy.value) return
        signerPackages = availablePackages()
        if (signerPackages.isEmpty()) {
            showError("Install a signer app such as Amber, then try again.")
            return
        }
        mutableBusy.value = true
        mutableRequest.value =
            SignerActivityRequest(
                id = UUID.randomUUID().toString(),
                type = "get_public_key",
                packageName = signerPackages.singleOrNull(),
            )
        timeout = scope.launch {
            delay(180_000)
            fail("Sign-in timed out. Please try again.")
        }
    }

    /** Claim before launching so collection after rotation/resume cannot open the signer twice. */
    @Synchronized
    fun claimRequest(id: String): SignerActivityRequest? {
        val request = mutableRequest.value ?: return null
        if (request.id != id || launchedRequestId == id) return null
        launchedRequestId = id
        return request
    }

    @Synchronized
    fun signEvent(update: AppUpdate.SignerLoginSignEvent) {
        if (!mutableBusy.value || !waitingForCore || selectedPubkey != update.ownerPubkeyHex) {
            dispatch(AppAction.CancelSignerLogin(update.requestId))
            return
        }
        coreRequestId = update.requestId
        mutableRequest.value =
            SignerActivityRequest(
                id = update.requestId,
                type = "sign_event",
                payload = update.unsignedEventJson,
                packageName = selectedPackage,
                ownerPubkeyHex = update.ownerPubkeyHex,
            )
    }

    @Synchronized
    fun onAppState(state: AppState) {
        if (!waitingForCore) return
        if (state.busy.restoringSession) observedCoreBusy = true
        // FullState updates can coalesce, including the initial busy=true snapshot on a fast
        // lookup failure. Core failures always include a toast, so those also end this exchange.
        if (!state.busy.restoringSession && (observedCoreBusy || state.account != null || state.toast != null)) clear()
    }

    @Synchronized
    fun onActivityResult(requestId: String, resultCode: Int, data: Intent?) {
        val request = mutableRequest.value
        if (request == null || request.id != requestId || launchedRequestId != request.id) {
            // Android can deliver an old result after process death. There is no pending Rust
            // authorization to match it against, so it must never authorize a new device.
            return
        }
        if (resultCode != Activity.RESULT_OK || data?.getBooleanExtra("rejected", false) == true) {
            fail("Sign-in canceled.")
            return
        }
        val responseId = data?.getStringExtra("id")
        if (responseId != null && responseId != request.id) {
            fail("The signer returned an unexpected response. Please try again.")
            return
        }
        if (request.type == "get_public_key") {
            receivePublicKey(request, data)
        } else {
            receiveSignedEvent(request, data)
        }
    }

    @Synchronized
    fun launchFailed(id: String) {
        if (mutableRequest.value?.id == id) fail("Couldn’t open the signer app. Please try again.")
    }

    @Synchronized
    fun cancel() {
        if (!mutableBusy.value) return
        val requestId = coreRequestId.orEmpty()
        val cancelCore = waitingForCore
        clear()
        if (cancelCore) dispatch(AppAction.CancelSignerLogin(requestId))
    }

    private fun receivePublicKey(request: SignerActivityRequest, data: Intent?) {
        val packageName = data?.getStringExtra("package").orEmpty()
        val publicKeyInput = data?.getStringExtra("result").orEmpty().trim()
        // Older Amber versions return npub. Accept public-key forms only, never secret keys or
        // arbitrary identity links accepted by the broader chat-address parser.
        val pubkey =
            if (publicKeyInput.matches(Regex("(?i)[0-9a-f]{64}|npub1[023456789acdefghjklmnpqrstuvwxyz]+"))) {
                peerInputToHex(publicKeyInput).lowercase()
            } else {
                ""
            }
        if (
            packageName !in signerPackages ||
            (request.packageName != null && packageName != request.packageName) ||
            !pubkey.matches(Regex("[0-9a-f]{64}"))
        ) {
            fail("The signer returned an invalid account. Please try again.")
            return
        }
        selectedPackage = packageName
        selectedPubkey = pubkey
        mutableRequest.value = null
        waitingForCore = true
        if (!dispatch(AppAction.BeginSignerLogin(pubkey))) {
            fail("Couldn’t sign in. Please try again.")
        }
    }

    private fun receiveSignedEvent(request: SignerActivityRequest, data: Intent?) {
        val returnedPackage = data?.getStringExtra("package")
        if (returnedPackage != null && returnedPackage != selectedPackage) {
            fail("The signer returned an unexpected response. Please try again.")
            return
        }
        val signedEvent =
            runCatching {
                data?.getStringExtra("event")?.takeIf { it.isNotBlank() }
                    ?: data?.getStringExtra("result")?.takeIf { it.isNotBlank() }?.let { result ->
                        if (result.startsWith("{")) result else JSONObject(request.payload).put("sig", result).toString()
                    }
            }.getOrNull()
        if (signedEvent == null) {
            fail("The signer didn’t return a signature. Please try again.")
            return
        }
        mutableRequest.value = null
        // Rust verifies the signature, identity, and complete event against its pending request.
        if (!dispatch(AppAction.CompleteSignerLogin(request.id, signedEvent))) {
            fail("Couldn’t sign in. Please try again.")
        } else {
            // Once submitted, core owns the bounded publication attempt. Do not time out the
            // shell and invite a retry while a valid device authorization is being published.
            timeout?.cancel()
            timeout = null
        }
    }

    @Synchronized
    private fun fail(message: String) {
        cancel()
        showError(message)
    }

    private fun clear() {
        timeout?.cancel()
        timeout = null
        mutableRequest.value = null
        mutableBusy.value = false
        launchedRequestId = null
        signerPackages = emptySet()
        selectedPackage = null
        selectedPubkey = null
        coreRequestId = null
        waitingForCore = false
        observedCoreBusy = false
    }

    @Suppress("DEPRECATION")
    private fun availablePackages(): Set<String> =
        context.packageManager
            .queryIntentActivities(Intent(Intent.ACTION_VIEW, Uri.parse("nostrsigner:")), PackageManager.MATCH_DEFAULT_ONLY)
            .map { it.activityInfo.packageName }
            .toSet()
}
