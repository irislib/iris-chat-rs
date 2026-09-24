package to.iris.chat.core

import to.iris.chat.BuildConfig
import to.iris.chat.rust.AppState

internal data class AndroidMobilePushSyncInput(
    val enabled: Boolean,
    val ownerPubkeyHex: String?,
    val ownerSecretAvailable: Boolean,
    val messageAuthorPubkeys: List<String>,
    val backgroundMessageAuthorPubkeys: List<String>,
    val inviteResponsePubkeys: List<String>,
    val serverOverride: String,
    val callDevicePubkeyHex: String?,
    val callAuthorPubkeys: List<String>,
    val callsEnabled: Boolean,
)

internal fun mobilePushSyncInput(
    state: AppState,
    ownerNsec: String?,
): AndroidMobilePushSyncInput =
    AndroidMobilePushSyncInput(
        enabled = state.preferences.desktopNotificationsEnabled,
        callDevicePubkeyHex = state.mobilePush.callDevicePubkeyHex,
        callAuthorPubkeys = state.mobilePush.callAuthorPubkeys,
        callsEnabled = state.preferences.voiceCallsEnabled || state.preferences.videoCallsEnabled,
        ownerPubkeyHex = state.mobilePush.ownerPubkeyHex?.trim()?.ifEmpty { null },
        ownerSecretAvailable = !ownerNsec.isNullOrBlank(),
        messageAuthorPubkeys = state.mobilePush.messageAuthorPubkeys,
        backgroundMessageAuthorPubkeys = state.mobilePush.backgroundMessageAuthorPubkeys,
        inviteResponsePubkeys = state.mobilePush.inviteResponsePubkeys,
        serverOverride =
            state.preferences.mobilePushServerUrl
                .trim()
                .ifEmpty { BuildConfig.MOBILE_PUSH_SERVER_URL.trim() },
    )
