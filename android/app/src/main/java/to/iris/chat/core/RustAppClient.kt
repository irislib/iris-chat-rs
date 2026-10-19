package to.iris.chat.core

import android.content.Context
import to.iris.chat.IrisDebugLog
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AppState
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DirectFileDestination
import to.iris.chat.rust.FfiApp
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.nearby.IrisFipsBleRuntime
import to.iris.chat.nearby.NearbyBleSession
import to.iris.chat.nearby.NearbyBleUpdates

interface RustAppClient {
    fun state(): AppState

    fun userProfile(ownerPubkeyHex: String): to.iris.chat.rust.UserProfileSnapshot =
        to.iris.chat.rust.UserProfileSnapshot(ownerPubkeyHex,
            to.iris.chat.rust.presentPersonName("", ownerPubkeyHex, null), null, null)

    fun dispatch(action: AppAction)

    fun acceptDirectFiles(chatId: String, transferId: String, destination: DirectFileDestination)

    fun search(query: String, scopeChatId: String?, limit: UInt): SearchResultSnapshot

    fun chatSnapshot(chatId: String, limit: UInt): CurrentChatSnapshot?

    fun chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt): CurrentChatSnapshot?

    fun chatSnapshotAroundMessage(
        chatId: String,
        messageId: String,
        beforeLimit: UInt,
        afterLimit: UInt,
    ): CurrentChatSnapshot?

    fun mutualGroups(ownerInput: String): List<ChatThreadSnapshot>

    fun exportSupportBundleJson(): String

    fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot?

    fun prepareForSuspend()

    fun setFipsBleEnabled(enabled: Boolean, nearbyLanEnabled: Boolean) = Unit

    fun listenForUpdates(reconciler: AppReconciler)

    fun shutdown()
}

internal class LiveRustAppClient(
    private val context: Context,
    dataDir: String,
    appVersion: String,
) : RustAppClient {
    private val ffi = FfiApp(dataDir = dataDir, keychainGroup = "", appVersion = appVersion)
    private val fipsBle = NearbyBleSession { IrisFipsBleRuntime(context, ffi) }
    private val fipsBleUpdates = NearbyBleUpdates(fipsBle::update) { error ->
        IrisDebugLog.d("FipsBle", "FIPS BLE could not update", error)
    }

    override fun state(): AppState = ffi.state()

    override fun userProfile(ownerPubkeyHex: String) = ffi.userProfile(ownerPubkeyHex)

    override fun dispatch(action: AppAction) {
        ffi.dispatch(action)
    }

    override fun acceptDirectFiles(chatId: String, transferId: String, destination: DirectFileDestination) {
        ffi.acceptDirectFiles(chatId, transferId, destination)
    }

    override fun search(query: String, scopeChatId: String?, limit: UInt): SearchResultSnapshot =
        ffi.search(query = query, scopeChatId = scopeChatId, limit = limit)

    override fun chatSnapshot(chatId: String, limit: UInt): CurrentChatSnapshot? =
        ffi.chatSnapshot(chatId = chatId, limit = limit)

    override fun chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt): CurrentChatSnapshot? =
        ffi.chatSnapshotBefore(chatId = chatId, beforeMessageId = beforeMessageId, limit = limit)

    override fun chatSnapshotAroundMessage(
        chatId: String,
        messageId: String,
        beforeLimit: UInt,
        afterLimit: UInt,
    ): CurrentChatSnapshot? =
        ffi.chatSnapshotAroundMessage(
            chatId = chatId,
            messageId = messageId,
            beforeLimit = beforeLimit,
            afterLimit = afterLimit,
        )

    override fun mutualGroups(ownerInput: String): List<ChatThreadSnapshot> =
        ffi.mutualGroups(ownerInput = ownerInput).groups

    override fun exportSupportBundleJson(): String = ffi.exportSupportBundleJson()

    override fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot? =
        ffi.peerProfileDebug(ownerInput = ownerInput)

    override fun prepareForSuspend() {
        ffi.prepareForSuspend()
    }

    override fun setFipsBleEnabled(enabled: Boolean, nearbyLanEnabled: Boolean) {
        fipsBleUpdates.update(enabled, nearbyLanEnabled)
    }

    override fun listenForUpdates(reconciler: AppReconciler) {
        ffi.listenForUpdates(reconciler)
    }

    override fun shutdown() {
        try {
            try {
                fipsBleUpdates.close()
            } finally {
                fipsBle.close()
            }
        } finally {
            // Both reset callers run on ioDispatcher. Keep the old platform
            // alive until the core worker has actually exited, not just ACKed.
            fipsBle.shutdownCoreAndCleanup(ffi::shutdownAndWait) { it.stopPlatform() }
        }
    }
}

