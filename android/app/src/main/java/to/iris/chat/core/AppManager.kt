package to.iris.chat.core

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.util.Base64
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStoreFile
import java.io.IOException
import java.io.File
import java.util.Collections
import java.util.Locale
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import to.iris.chat.BuildConfig
import to.iris.chat.IrisDebugLog
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.account.AccountState
import to.iris.chat.account.AndroidKeystoreSecretStore
import to.iris.chat.account.EncryptedSecret
import to.iris.chat.account.SecureSecretStore
import to.iris.chat.account.StoredAccountBundle
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AccountSnapshot
import to.iris.chat.rust.AppState
import to.iris.chat.rust.BusyState
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DeviceRosterSnapshot
import to.iris.chat.rust.GroupDetailsSnapshot
import to.iris.chat.rust.NetworkStatusSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.PublicInviteSnapshot
import to.iris.chat.rust.Router
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.FfiApp
import to.iris.chat.rust.MessageAttachmentSnapshot
import to.iris.chat.rust.OutgoingAttachment
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.downloadHashtreeAttachment
import to.iris.chat.push.AndroidMobilePushRuntime
import to.iris.chat.update.AndroidSelfUpdateManager

interface RustAppClient {
    fun state(): AppState

    fun dispatch(action: AppAction)

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

    fun ingestNearbyEventJson(eventJson: String): Boolean

    fun ingestNearbyEventJsonWithTransport(eventJson: String, transport: String): Boolean

    fun buildNearbyPresenceEventJson(
        peerId: String,
        myNonce: String,
        theirNonce: String,
        profileEventId: String,
    ): String

    fun verifyNearbyPresenceEventJson(
        eventJson: String,
        peerId: String,
        myNonce: String,
        theirNonce: String,
    ): String

    fun nearbyEncodeFrame(envelopeJson: String): ByteArray

    fun nearbyDecodeFrame(frame: ByteArray): String

    fun nearbyFrameBodyLenFromHeader(header: ByteArray): Int

    fun exportSupportBundleJson(): String

    fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot?

    fun prepareForSuspend()

    fun listenForUpdates(reconciler: AppReconciler)

    fun shutdown()
}

private class LiveRustAppClient(
    dataDir: String,
    appVersion: String,
) : RustAppClient {
    private val ffi = FfiApp(dataDir = dataDir, keychainGroup = "", appVersion = appVersion)

    override fun state(): AppState = ffi.state()

    override fun dispatch(action: AppAction) {
        ffi.dispatch(action)
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

    override fun ingestNearbyEventJson(eventJson: String): Boolean = ffi.ingestNearbyEventJson(eventJson)

    override fun ingestNearbyEventJsonWithTransport(eventJson: String, transport: String): Boolean =
        ffi.ingestNearbyEventJsonWithTransport(eventJson, transport)

    override fun buildNearbyPresenceEventJson(
        peerId: String,
        myNonce: String,
        theirNonce: String,
        profileEventId: String,
    ): String =
        ffi.buildNearbyPresenceEventJson(
            peerId = peerId,
            myNonce = myNonce,
            theirNonce = theirNonce,
            profileEventId = profileEventId,
        )

    override fun verifyNearbyPresenceEventJson(
        eventJson: String,
        peerId: String,
        myNonce: String,
        theirNonce: String,
    ): String =
        ffi.verifyNearbyPresenceEventJson(
            eventJson = eventJson,
            peerId = peerId,
            myNonce = myNonce,
            theirNonce = theirNonce,
        )

    override fun nearbyEncodeFrame(envelopeJson: String): ByteArray =
        ffi.nearbyEncodeFrame(envelopeJson = envelopeJson)

    override fun nearbyDecodeFrame(frame: ByteArray): String =
        ffi.nearbyDecodeFrame(frame = frame)

    override fun nearbyFrameBodyLenFromHeader(header: ByteArray): Int =
        ffi.nearbyFrameBodyLenFromHeader(header = header)

    override fun exportSupportBundleJson(): String = ffi.exportSupportBundleJson()

    override fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot? =
        ffi.peerProfileDebug(ownerInput = ownerInput)

    override fun prepareForSuspend() {
        ffi.prepareForSuspend()
    }

    override fun listenForUpdates(reconciler: AppReconciler) {
        ffi.listenForUpdates(reconciler)
    }

    override fun shutdown() {
        ffi.shutdown()
    }
}

private const val MOBILE_PUSH_GROUP_CHAT_PREFIX = "group:"
private const val ACTIVE_CHAT_SEEN_IDLE_LIMIT_SECS = 5 * 60L

private fun activeNotificationChatIds(
    currentChat: CurrentChatSnapshot?,
    router: Router,
): Set<String> =
    buildSet {
        val activeScreen = router.screenStack.lastOrNull() ?: router.defaultScreen
        if (activeScreen is Screen.Chat) {
            normalizedNotificationId(activeScreen.chatId)?.let(::add)
        }
        normalizedNotificationId(currentChat?.chatId.orEmpty())?.let(::add)
        currentChat?.groupId
            ?.let(::normalizedNotificationId)
            ?.let { groupId ->
                add("$MOBILE_PUSH_GROUP_CHAT_PREFIX$groupId")
            }
    }

private fun pushNotificationChatCandidates(payload: JSONObject): Set<String> =
    buildSet {
        listOf(
            "chat_id",
            "chatId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
            "sender_pubkey",
            "senderPubkey",
            "author_pubkey",
            "authorPubkey",
        ).forEach { key ->
            normalizedNotificationId(payload.optString(key))?.let(::add)
        }
        listOf("group_id", "groupId", "group_chat_id", "groupChatId").forEach { key ->
            normalizedNotificationId(payload.optString(key))?.let { groupId ->
                if (groupId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX)) {
                    add(groupId)
                } else {
                    add("$MOBILE_PUSH_GROUP_CHAT_PREFIX$groupId")
                }
            }
        }
    }

private fun notificationChatIdMatches(
    activeChatId: String,
    pushChatId: String,
): Boolean {
    if (activeChatId == pushChatId) {
        return true
    }
    val activeGroupId = activeChatId.removePrefix(MOBILE_PUSH_GROUP_CHAT_PREFIX)
        .takeIf { activeChatId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX) }
    val pushGroupId = pushChatId.removePrefix(MOBILE_PUSH_GROUP_CHAT_PREFIX)
        .takeIf { pushChatId.startsWith(MOBILE_PUSH_GROUP_CHAT_PREFIX) }
    return when {
        activeGroupId != null && pushGroupId != null -> activeGroupId == pushGroupId
        activeGroupId != null -> activeGroupId == pushChatId
        pushGroupId != null -> activeChatId == pushGroupId
        else -> false
    }
}

private fun normalizedNotificationId(value: String): String? =
    value
        .trim()
        .takeIf { it.isNotEmpty() }
        ?.lowercase(Locale.ROOT)

data class NearbyPublishedEvent(
    val eventId: String,
    val kind: UInt,
    val createdAtSecs: ULong,
    val eventJson: String,
)

data class PendingShare(
    val text: String,
    val attachments: List<OutgoingAttachment>,
    val isForward: Boolean = false,
)

private data class PendingNavigationOverride(
    val stack: List<Screen>,
    val expiresAtMs: Long,
)

private data class AndroidMobilePushSyncInput(
    val enabled: Boolean,
    val ownerPubkeyHex: String?,
    val ownerSecretAvailable: Boolean,
    val messageAuthorPubkeys: List<String>,
    val inviteResponsePubkeys: List<String>,
    val serverOverride: String,
)

private data class ClientDebugLogEntry(
    val timestampSecs: Long,
    val category: String,
    val detail: String,
) {
    fun toJsonObject(): JSONObject =
        JSONObject()
            .put("timestamp_secs", timestampSecs)
            .put("category", category)
            .put("detail", detail)
}

class AppManager(
    context: Context,
    private val applicationScope: CoroutineScope,
    private val secureSecretStore: SecureSecretStore = AndroidKeystoreSecretStore(),
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
    dataStoreName: String = DATASTORE_NAME,
    dataStore: DataStore<Preferences>? = null,
    private val rustFactory: ((dataDir: String, appVersion: String) -> RustAppClient)? = null,
) {
    private val appContext = context.applicationContext
    private val rustDataDir = appContext.filesDir.absolutePath
    private val dataStore =
        dataStore
            ?: PreferenceDataStoreFactory.create(
                produceFile = { appContext.preferencesDataStoreFile(dataStoreName) },
            )
    private val mobilePushRuntime = AndroidMobilePushRuntime(this.dataStore)
    private val selfUpdateManager =
        AndroidSelfUpdateManager(
            context = appContext,
            dataStore = this.dataStore,
            scope = applicationScope,
            ioDispatcher = ioDispatcher,
        )

    private var rust = createRustApp()
    private var rustGeneration: Long = 0
    @Volatile
    private var nearbyEventPublisher: ((NearbyPublishedEvent) -> Unit)? = null
    private val nearbyPublishMutex = Mutex()
    @Volatile
    private var appInForeground: Boolean = false

    private var lastRevApplied: ULong = 0u
    private var restoreCheckComplete = false
    private var persistedRestoreInFlight = false
    private var cachedAccountBundle: StoredAccountBundle? = null
    private var lastMobilePushSyncInput: AndroidMobilePushSyncInput? = null
    private var lastSyncedDeviceLabelsKey: String? = null
    private var pendingNavigationOverride: PendingNavigationOverride? = null
    private val olderChatPageLoads = Collections.synchronizedSet(mutableSetOf<String>())
    private val exhaustedOlderChatPages = Collections.synchronizedSet(mutableSetOf<String>())
    private val aroundChatPageLoads = Collections.synchronizedSet(mutableSetOf<String>())
    private val clientDebugLog = Collections.synchronizedList(mutableListOf<ClientDebugLogEntry>())

    private val mutableState = MutableStateFlow(rust.state())
    private val mutableAppForegrounded = MutableStateFlow(false)
    private val mutableForegroundedAtSecs = MutableStateFlow(currentTimeSeconds())
    private val mutableLastUserActivityAtSecs = MutableStateFlow(currentTimeSeconds())

    /**
     * Whole-state flow for callers that genuinely need the consolidated
     * snapshot (notification side effects, ad-hoc reads). **Composable
     * screens should subscribe to one of the slice flows below instead**
     * because `state.collectAsStateWithLifecycle()` recomposes on every
     * relay event, even those that don't change anything the screen renders.
     */
    val state: StateFlow<AppState> = mutableState.asStateFlow()
    val appForegrounded: StateFlow<Boolean> = mutableAppForegrounded.asStateFlow()
    val foregroundedAtSecs: StateFlow<Long> = mutableForegroundedAtSecs.asStateFlow()
    val lastUserActivityAtSecs: StateFlow<Long> = mutableLastUserActivityAtSecs.asStateFlow()

    // Per-slice flows. Each derives from `mutableState` via
    // `map { ... }.distinctUntilChanged()` so a Compose subscriber only
    // recomposes when its specific slice actually changed. This is what
    // turns a backlog of relay events from a multi-second UI freeze into
    // imperceptible updates: ChatScreen no longer recomposes when only
    // chat_list changes, ChatListScreen doesn't recompose when only
    // current_chat changes, etc.
    val router: StateFlow<Router> = slice("router") { it.router }
    val account: StateFlow<AccountSnapshot?> = slice("account") { it.account }
    val deviceRoster: StateFlow<DeviceRosterSnapshot?> =
        slice("deviceRoster") { it.deviceRoster }
    val busy: StateFlow<BusyState> = slice("busy") { it.busy }
    val chatList: StateFlow<List<ChatThreadSnapshot>> =
        slice("chatList") { it.chatList }
    val currentChat: StateFlow<CurrentChatSnapshot?> =
        slice("currentChat") { it.currentChat }
    val groupDetails: StateFlow<GroupDetailsSnapshot?> =
        slice("groupDetails") { it.groupDetails }
    val publicInvite: StateFlow<PublicInviteSnapshot?> =
        slice("publicInvite") { it.publicInvite }
    val networkStatus: StateFlow<NetworkStatusSnapshot?> =
        slice("networkStatus") { it.networkStatus }
    val preferences: StateFlow<PreferencesSnapshot> =
        slice("preferences") { it.preferences }
    val toast: StateFlow<String?> = slice("toast") { it.toast }
    val selfUpdateState = selfUpdateManager.state
    private val mutablePendingShare = MutableStateFlow<PendingShare?>(null)
    val pendingShare: StateFlow<PendingShare?> = mutablePendingShare.asStateFlow()

    @Suppress("unused") // tag is helpful for tracing during perf work
    private fun <T> slice(
        @Suppress("UNUSED_PARAMETER") tag: String,
        select: (AppState) -> T,
    ): StateFlow<T> =
        mutableState
            .map(select)
            .distinctUntilChanged()
            .stateIn(
                scope = applicationScope,
                started = SharingStarted.Eagerly,
                initialValue = select(mutableState.value),
            )

    private val mutableBootstrapState =
        MutableStateFlow<AccountBootstrapState>(AccountBootstrapState.Loading)
    val bootstrapState: StateFlow<AccountBootstrapState> = mutableBootstrapState.asStateFlow()

    init {
        val initial = bindRust(rust)
        IrisDebugLog.enabled = initial.preferences.debugLoggingEnabled
        IrisDebugLog.d(TAG, "init rev=${initial.rev} defaultScreen=${initial.router.defaultScreen}")
        publishState(initial)
        applicationScope.launch(ioDispatcher) {
            restoreSessionFromSecureStore()
        }
    }

    fun createAccount() {
        createAccount("")
    }

    fun createAccount(name: String) {
        dispatchToRust(AppAction.CreateAccount(name.trim()))
    }

    fun updateProfileMetadata(
        name: String,
        pictureUrl: String?,
        about: String?,
    ) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatchToRust(
            AppAction.UpdateProfileMetadata(
                name = trimmed,
                pictureUrl = pictureUrl?.trim()?.ifEmpty { null },
                about = about?.trim()?.ifEmpty { null },
            ),
        )
    }

    fun uploadProfilePicture(filePath: String) {
        val trimmedPath = filePath.trim()
        if (trimmedPath.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.UploadProfilePicture(trimmedPath))
    }

    fun restoreSession(nsecOrHex: String) {
        val trimmed = nsecOrHex.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.RestoreSession(trimmed))
    }

    fun startLinkedDevice(ownerInput: String) {
        dispatchToRust(AppAction.StartLinkedDevice(ownerInput.trim()))
    }

    fun addAuthorizedDevice(deviceInput: String) {
        val trimmed = deviceInput.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.AddAuthorizedDevice(trimmed))
    }

    fun removeAuthorizedDevice(devicePubkeyHex: String) {
        val trimmed = devicePubkeyHex.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.RemoveAuthorizedDevice(trimmed))
    }

    fun dispatch(action: AppAction) {
        if (handleOptimisticNavigation(action)) {
            return
        }
        dispatchToRust(action)
    }

    /**
     * Grouped search: contacts + groups filtered from the in-memory
     * chat list, plus message hits from the SQLite FTS5 index. Cheap
     * enough to call on every keystroke; runs synchronously on the
     * caller thread, so the chat-list view should hop to the IO
     * dispatcher when binding it to a `TextField`.
     */
    fun search(query: String, scopeChatId: String? = null, limit: UInt = 50u): SearchResultSnapshot =
        runCatching { rust.search(query, scopeChatId, limit) }.getOrElse {
            logFfiFailure(
                category = "ffi.search.failed",
                detail = "query_len=${query.length} scoped=${scopeChatId != null}",
                error = it,
            )
            SearchResultSnapshot(
                query = query,
                scopeChatId = scopeChatId,
                contacts = emptyList(),
                groups = emptyList(),
                messages = emptyList(),
                shortcut = null,
            )
        }

    fun setNearbyEventPublisher(publisher: ((NearbyPublishedEvent) -> Unit)?) {
        nearbyEventPublisher = publisher
    }

    fun ingestNearbyEventJson(eventJson: String): Boolean =
        runCatching { rust.ingestNearbyEventJson(eventJson) }.getOrDefault(false)

    fun ingestNearbyEventJsonWithTransport(eventJson: String, transport: String): Boolean =
        runCatching { rust.ingestNearbyEventJsonWithTransport(eventJson, transport) }.getOrDefault(false)

    fun buildNearbyPresenceEventJson(
        peerId: String,
        myNonce: String,
        theirNonce: String,
        profileEventId: String?,
    ): String =
        runCatching {
            rust.buildNearbyPresenceEventJson(
                peerId = peerId,
                myNonce = myNonce,
                theirNonce = theirNonce,
                profileEventId = profileEventId.orEmpty(),
            )
        }.getOrDefault("")

    fun verifyNearbyPresenceEventJson(
        eventJson: String,
        peerId: String,
        myNonce: String,
        theirNonce: String,
    ): String =
        runCatching {
            rust.verifyNearbyPresenceEventJson(
                eventJson = eventJson,
                peerId = peerId,
                myNonce = myNonce,
                theirNonce = theirNonce,
            )
        }.getOrDefault("")

    fun encodeNearbyFrame(envelope: JSONObject): ByteArray? =
        runCatching {
            rust.nearbyEncodeFrame(envelope.toString()).takeIf { it.isNotEmpty() }
        }.getOrNull()

    fun decodeNearbyFrame(frame: ByteArray): JSONObject? =
        runCatching {
            rust.nearbyDecodeFrame(frame).takeIf { it.isNotBlank() }?.let(::JSONObject)
        }.getOrNull()

    fun nearbyFrameBodyLenFromHeader(header: ByteArray): Int =
        runCatching { rust.nearbyFrameBodyLenFromHeader(header) }.getOrDefault(-1)

    fun appForegrounded() {
        appInForeground = true
        mutableAppForegrounded.value = true
        mutableForegroundedAtSecs.value = currentTimeSeconds()
        recordUserActivity()
        selfUpdateManager.startAutomaticChecks()
        dispatchToRust(AppAction.AppForegrounded, showsToastOnFailure = false)
    }

    fun appBackgrounded() {
        appInForeground = false
        mutableAppForegrounded.value = false
        selfUpdateManager.stopAutomaticChecks()
        runCatching {
            rust.prepareForSuspend()
        }.onFailure { error ->
            logFfiFailure("ffi.prepare_suspend.failed", "", error)
            Log.w(TAG, "failed to prepare Rust core for suspend", error)
        }
    }

    fun createChat(peerInput: String) {
        val trimmed = peerInput.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.CreateChat(trimmed))
    }

    fun createGroup(
        name: String,
        memberInputs: List<String>,
        pictureFilePath: String? = null,
        pictureFilename: String? = null,
    ) {
        val trimmedName = name.trim()
        val trimmedMembers = memberInputs.map(String::trim).filter(String::isNotEmpty)
        if (trimmedName.isEmpty()) {
            return
        }
        val trimmedPicturePath = pictureFilePath?.trim().orEmpty()
        val trimmedPictureFilename = pictureFilename?.trim().orEmpty()
        if (trimmedPicturePath.isNotEmpty() && trimmedPictureFilename.isNotEmpty()) {
            dispatchToRust(
                AppAction.CreateGroupWithPicture(
                    trimmedName,
                    trimmedMembers,
                    trimmedPicturePath,
                    trimmedPictureFilename,
                ),
            )
        } else {
            dispatchToRust(AppAction.CreateGroup(trimmedName, trimmedMembers))
        }
    }

    fun recordUserActivity() {
        val now = currentTimeSeconds()
        if (now != mutableLastUserActivityAtSecs.value) {
            mutableLastUserActivityAtSecs.value = now
        }
    }

    fun canMarkActiveChatSeen(
        appForegrounded: Boolean = this.appInForeground,
        lastUserActivityAtSecs: Long = this.mutableLastUserActivityAtSecs.value,
    ): Boolean =
        appForegrounded &&
            currentTimeSeconds() - lastUserActivityAtSecs <= ACTIVE_CHAT_SEEN_IDLE_LIMIT_SECS

    fun updateGroupName(
        groupId: String,
        name: String,
    ) {
        val trimmedGroupId = groupId.trim()
        val trimmedName = name.trim()
        if (trimmedGroupId.isEmpty() || trimmedName.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.UpdateGroupName(trimmedGroupId, trimmedName))
    }

    fun updateGroupPicture(
        groupId: String,
        filePath: String,
        filename: String,
    ) {
        val trimmedGroupId = groupId.trim()
        val trimmedPath = filePath.trim()
        val trimmedFilename = filename.trim()
        if (trimmedGroupId.isEmpty() || trimmedPath.isEmpty() || trimmedFilename.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.UpdateGroupPicture(trimmedGroupId, trimmedPath, trimmedFilename))
    }

    fun addGroupMembers(
        groupId: String,
        memberInputs: List<String>,
    ) {
        val trimmedGroupId = groupId.trim()
        val trimmedMembers = memberInputs.map(String::trim).filter(String::isNotEmpty)
        if (trimmedGroupId.isEmpty() || trimmedMembers.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.AddGroupMembers(trimmedGroupId, trimmedMembers))
    }

    fun removeGroupMember(
        groupId: String,
        ownerPubkeyHex: String,
    ) {
        val trimmedGroupId = groupId.trim()
        val trimmedOwner = ownerPubkeyHex.trim()
        if (trimmedGroupId.isEmpty() || trimmedOwner.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.RemoveGroupMember(trimmedGroupId, trimmedOwner))
    }

    fun setGroupAdmin(
        groupId: String,
        ownerPubkeyHex: String,
        isAdmin: Boolean,
    ) {
        val trimmedGroupId = groupId.trim()
        val trimmedOwner = ownerPubkeyHex.trim()
        if (trimmedGroupId.isEmpty() || trimmedOwner.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.SetGroupAdmin(trimmedGroupId, trimmedOwner, isAdmin))
    }

    fun openChat(chatId: String) {
        val trimmed = chatId.trim()
        if (trimmed.isEmpty()) {
            return
        }
        dispatch(AppAction.OpenChat(trimmed))
    }

    /// Open a chat with a one-shot "scroll to this message" hint.
    /// ChatScreen consumes it on first paint and resets via
    /// `consumePendingScrollMessage`.
    fun openChatAtMessage(chatId: String, messageId: String) {
        val trimmedChat = chatId.trim()
        val trimmedMessage = messageId.trim()
        if (trimmedChat.isEmpty() || trimmedMessage.isEmpty()) {
            return
        }
        pendingScrollMessageState.value = trimmedMessage
        dispatch(AppAction.OpenChat(trimmedChat))
        loadChatAroundMessage(trimmedChat, trimmedMessage)
    }

    fun consumePendingScrollMessage() {
        if (pendingScrollMessageState.value != null) {
            pendingScrollMessageState.value = null
        }
    }

    private val pendingScrollMessageState = MutableStateFlow<String?>(null)
    val pendingScrollMessage: StateFlow<String?> = pendingScrollMessageState.asStateFlow()

    fun loadOlderMessages(chatId: String): Boolean {
        val trimmedChat = chatId.trim()
        val current = mutableState.value.currentChat
        val firstMessage = current?.messages?.firstOrNull()
        if (
            trimmedChat.isEmpty() ||
                trimmedChat in exhaustedOlderChatPages ||
                current?.chatId != trimmedChat ||
                firstMessage == null
        ) {
            return false
        }
        synchronized(olderChatPageLoads) {
            if (trimmedChat in olderChatPageLoads) {
                return true
            }
            olderChatPageLoads += trimmedChat
        }
        val firstMessageId = firstMessage.id
        applicationScope.launch(ioDispatcher) {
            val page =
                runCatching {
                    rust.chatSnapshotBefore(trimmedChat, firstMessageId, CHAT_PAGE_SIZE)
                }.getOrNull()
            synchronized(olderChatPageLoads) {
                olderChatPageLoads -= trimmedChat
            }
            if (page == null) {
                return@launch
            }
            if (page.messages.isEmpty() || page.messages.size.toUInt() < CHAT_PAGE_SIZE) {
                exhaustedOlderChatPages += trimmedChat
            }
            if (page.messages.isNotEmpty()) {
                mergeCurrentChatSnapshot(page)
            }
        }
        return true
    }

    fun loadChatAroundMessage(chatId: String, messageId: String) {
        val trimmedChat = chatId.trim()
        val trimmedMessage = messageId.trim()
        if (trimmedChat.isEmpty() || trimmedMessage.isEmpty()) {
            return
        }
        val current = mutableState.value.currentChat
        if (current?.chatId == trimmedChat && current.messages.any { it.id == trimmedMessage }) {
            return
        }
        val key = "$trimmedChat\u001F$trimmedMessage"
        synchronized(aroundChatPageLoads) {
            if (key in aroundChatPageLoads) {
                return
            }
            aroundChatPageLoads += key
        }
        applicationScope.launch(ioDispatcher) {
            val page =
                runCatching {
                    rust.chatSnapshotAroundMessage(
                        chatId = trimmedChat,
                        messageId = trimmedMessage,
                        beforeLimit = CHAT_AROUND_BEFORE_LIMIT,
                        afterLimit = CHAT_AROUND_AFTER_LIMIT,
                    )
                }.getOrNull()
            synchronized(aroundChatPageLoads) {
                aroundChatPageLoads -= key
            }
            if (page != null) {
                mergeCurrentChatSnapshot(page)
            }
        }
    }

    fun pushScreen(screen: Screen) {
        dispatch(AppAction.PushScreen(screen))
    }

    fun navigateBack() {
        val currentStack = mutableState.value.router.screenStack
        if (currentStack.isEmpty()) {
            return
        }
        val nextStack = currentStack.dropLast(1)
        navigateOptimistically(
            stack = nextStack,
            action = AppAction.UpdateScreenStack(nextStack),
            showsToastOnFailure = false,
        )
    }

    private fun handleOptimisticNavigation(action: AppAction): Boolean =
        when (action) {
            is AppAction.NavigateBack -> {
                navigateBack()
                true
            }
            is AppAction.OpenChat -> {
                val trimmed = action.chatId.trim()
                if (trimmed.isNotEmpty()) {
                    navigateOptimistically(
                        stack = listOf(Screen.Chat(trimmed)),
                        action = AppAction.OpenChat(trimmed),
                    )
                }
                true
            }
            is AppAction.PushScreen -> {
                val stack = stackByApplyingPushScreen(action.screen)
                if (stack == null) {
                    dispatchToRust(action)
                } else {
                    navigateOptimistically(stack = stack, action = action)
                }
                true
            }
            is AppAction.UpdateScreenStack -> {
                navigateOptimistically(
                    stack = action.stack,
                    action = action,
                    showsToastOnFailure = false,
                )
                true
            }
            else -> false
        }

    private fun stackByApplyingPushScreen(screen: Screen): List<Screen>? {
        if (mutableState.value.account == null) {
            return when (screen) {
                is Screen.Welcome -> emptyList()
                is Screen.CreateAccount,
                is Screen.RestoreAccount,
                is Screen.AddDevice,
                -> listOf(screen)
                else -> null
            }
        }

        return when (screen) {
            is Screen.ChatList -> emptyList()
            is Screen.NewChat,
            is Screen.NewGroup,
            is Screen.CreateInvite,
            is Screen.JoinInvite,
            is Screen.Settings,
            is Screen.DeviceRoster,
            -> listOf(screen)
            is Screen.Chat -> {
                val trimmed = screen.chatId.trim()
                if (trimmed.isEmpty()) null else listOf(Screen.Chat(trimmed))
            }
            is Screen.DirectChatInfo -> {
                val trimmed = screen.chatId.trim()
                if (trimmed.isEmpty()) {
                    null
                } else {
                    val currentStack = mutableState.value.router.screenStack
                    val info = Screen.DirectChatInfo(trimmed)
                    if (currentStack.lastOrNull() == info) {
                        currentStack
                    } else {
                        currentStack + info
                    }
                }
            }
            is Screen.GroupDetails -> {
                val trimmed = screen.groupId.trim()
                if (trimmed.isEmpty()) {
                    null
                } else {
                    val groupChatId = "$MOBILE_PUSH_GROUP_CHAT_PREFIX$trimmed"
                    val current = mutableState.value
                    val activeScreen =
                        current.router.screenStack.lastOrNull() ?: current.router.defaultScreen
                    val routeChatId = (activeScreen as? Screen.Chat)?.chatId?.trim()
                    val currentChatId = current.currentChat?.chatId?.trim()
                    val baseStack =
                        if (routeChatId == groupChatId || currentChatId == groupChatId) {
                            current.router.screenStack
                        } else {
                            listOf(Screen.Chat(groupChatId))
                        }
                    val details = Screen.GroupDetails(trimmed)
                    if (baseStack.lastOrNull() == details) {
                        baseStack
                    } else {
                        baseStack + details
                    }
                }
            }
            is Screen.CreateAccount,
            is Screen.RestoreAccount,
            is Screen.AddDevice,
            is Screen.AwaitingDeviceApproval,
            is Screen.DeviceRevoked,
            is Screen.Welcome,
            -> null
        }
    }

    private fun navigateOptimistically(
        stack: List<Screen>,
        action: AppAction,
        showsToastOnFailure: Boolean = true,
    ) {
        pendingNavigationOverride =
            PendingNavigationOverride(
                stack = stack,
                expiresAtMs = SystemClock.elapsedRealtime() + NAVIGATION_OVERRIDE_TTL_MS,
            )
        publishState(stateByApplyingLocalScreenStack(stack, mutableState.value))
        val dispatched =
            dispatchToRust(
                action,
                showsToastOnFailure = showsToastOnFailure,
                preservesPendingNavigation = true,
            )
        if (!dispatched) {
            pendingNavigationOverride = null
        }
    }

    fun receiveShare(
        text: String,
        attachments: List<OutgoingAttachment>,
    ) {
        val trimmedText = text.trim()
        val outgoing =
            attachments
                .map {
                    OutgoingAttachment(
                        filePath = it.filePath.trim(),
                        filename = it.filename.trim(),
                    )
                }.filter { it.filePath.isNotEmpty() && it.filename.isNotEmpty() }
        if (trimmedText.isEmpty() && outgoing.isEmpty()) {
            return
        }
        mutablePendingShare.value = PendingShare(trimmedText, outgoing)
        dispatchToRust(AppAction.UpdateScreenStack(emptyList()))
    }

    fun startForward(text: String) {
        val trimmedText = text.trim()
        if (trimmedText.isEmpty()) {
            return
        }
        mutablePendingShare.value = PendingShare(trimmedText, emptyList(), isForward = true)
    }

    fun clearPendingShare() {
        mutablePendingShare.value = null
    }

    fun sendPendingShareToChat(chatId: String) {
        sendPendingShareToChats(listOf(chatId))
    }

    fun sendPendingShareToChats(chatIds: Collection<String>) {
        val share = mutablePendingShare.value ?: return
        val targets = chatIds.map { it.trim() }.filter { it.isNotEmpty() }.distinct()
        if (targets.isEmpty()) {
            return
        }
        targets.forEach { chatId ->
            if (share.attachments.isEmpty()) {
                sendText(chatId, share.text)
            } else {
                sendAttachments(chatId, share.attachments, share.text)
            }
        }
        openChat(targets.first())
        clearPendingShare()
    }

    fun sendText(
        chatId: String,
        text: String,
    ) {
        val trimmedChatId = chatId.trim()
        val trimmedText = text.trim()
        if (trimmedChatId.isEmpty() || trimmedText.isEmpty()) {
            return
        }
        dispatchToRust(AppAction.SendMessage(trimmedChatId, trimmedText))
    }

    fun sendAttachment(
        chatId: String,
        filePath: String,
        filename: String,
        caption: String,
    ) {
        val trimmedChatId = chatId.trim()
        val trimmedPath = filePath.trim()
        val trimmedFilename = filename.trim()
        if (trimmedChatId.isEmpty() || trimmedPath.isEmpty() || trimmedFilename.isEmpty()) {
            return
        }
        dispatchToRust(
            AppAction.SendAttachment(
                trimmedChatId,
                trimmedPath,
                trimmedFilename,
                caption.trim(),
            ),
        )
    }

    fun sendAttachments(
        chatId: String,
        attachments: List<OutgoingAttachment>,
        caption: String,
    ) {
        val trimmedChatId = chatId.trim()
        val outgoing =
            attachments
                .map {
                    OutgoingAttachment(
                        filePath = it.filePath.trim(),
                        filename = it.filename.trim(),
                    )
                }.filter { it.filePath.isNotEmpty() && it.filename.isNotEmpty() }
        if (trimmedChatId.isEmpty() || outgoing.isEmpty()) {
            return
        }
        dispatchToRust(
            AppAction.SendAttachments(
                trimmedChatId,
                outgoing,
                caption.trim(),
            ),
        )
    }

    suspend fun downloadAttachment(attachment: MessageAttachmentSnapshot): ByteArray? =
        withContext(ioDispatcher) {
            cachedDownloadedAttachment(attachment)?.let { return@withContext it }

            val result =
                downloadHashtreeAttachment(
                    nhash = attachment.nhash,
                )
            val data =
                result.dataBase64
                ?.takeIf(String::isNotBlank)
                ?.let { encoded -> Base64.decode(encoded, Base64.NO_WRAP) }
            if (data != null) {
                cacheDownloadedAttachment(attachment, data)
            }
            data
        }

    suspend fun downloadHashtreeBytes(nhash: String): ByteArray? =
        withContext(ioDispatcher) {
            val trimmed = nhash.trim()
            if (trimmed.isEmpty()) {
                return@withContext null
            }
            val result = downloadHashtreeAttachment(nhash = trimmed)
            result.dataBase64
                ?.takeIf(String::isNotBlank)
                ?.let { encoded -> Base64.decode(encoded, Base64.NO_WRAP) }
        }

    /**
     * Resolves an `htree://` profile picture (or any nhash) using the same
     * disk-backed cache that chat attachments use. Subsequent renders read
     * straight off disk instead of re-fetching from the hashtree network.
     */
    suspend fun resolveHashtreePictureBytes(nhash: String): ByteArray? =
        withContext(ioDispatcher) {
            val trimmed = nhash.trim()
            if (trimmed.isEmpty()) {
                return@withContext null
            }
            val cacheFile = pictureCacheFile(trimmed)
            if (cacheFile.isFile) {
                cacheFile.setLastModified(System.currentTimeMillis())
                runCatching { cacheFile.readBytes() }.getOrNull()?.let { return@withContext it }
            }
            val result = downloadHashtreeAttachment(nhash = trimmed)
            val data =
                result.dataBase64
                    ?.takeIf(String::isNotBlank)
                    ?.let { encoded -> Base64.decode(encoded, Base64.NO_WRAP) }
            if (data != null) {
                runCatching {
                    cacheFile.writeBytes(data)
                    pruneDownloadedAttachmentCache(protectedFile = cacheFile)
                }.onFailure { error ->
                    Log.w(TAG, "failed to cache profile picture", error)
                }
            }
            data
        }

    fun logout() {
        applicationScope.launch(ioDispatcher) {
            // Logout is owned by Rust. The shell clears native secrets and then swaps in a fresh core
            // instead of fabricating a shell-authored logged-out snapshot.
            val stateBeforeLogout = mutableState.value
            val persistedBundle = loadPersistedBundle()
            mobilePushRuntime.unregisterStoredSubscription(
                stateBeforeLogout,
                persistedBundle?.ownerNsec,
            )
            if (!clearNativeSecretsBeforeReset()) {
                return@launch
            }
            dispatchToRust(AppAction.Logout)
            replaceRustCoreAfterReset()
        }
    }

    suspend fun exportOwnerNsec(): String? =
        withContext(ioDispatcher) {
            loadPersistedBundle()?.ownerNsec
        }

    suspend fun exportDeviceNsec(): String? =
        withContext(ioDispatcher) {
            loadPersistedBundle()?.deviceNsec
        }

    suspend fun exportSupportBundleJson(): String =
        withContext(ioDispatcher) {
            val rustJson =
                runCatching { rust.exportSupportBundleJson() }.getOrElse { error ->
                    logFfiFailure("ffi.support_bundle.failed", "", error)
                    "{}"
                }
            supportBundleJsonWithClientLog(rustJson, snapshotClientDebugLog())
        }

    suspend fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot? =
        withContext(ioDispatcher) {
            runCatching { rust.peerProfileDebug(ownerInput) }.getOrElse { error ->
                logFfiFailure(
                    category = "ffi.peer_profile_debug.failed",
                    detail = "input_len=${ownerInput.length}",
                    error = error,
                )
                null
            }
        }

    suspend fun mutualGroups(ownerInput: String): List<ChatThreadSnapshot> =
        withContext(ioDispatcher) {
            runCatching { rust.mutualGroups(ownerInput) }.getOrElse { error ->
                logFfiFailure(
                    category = "ffi.mutual_groups.failed",
                    detail = "input_len=${ownerInput.length}",
                    error = error,
                )
                emptyList()
            }
        }

    fun resetAppState() {
        logout()
    }

    fun resetForUiTestsBlocking() {
        runBlocking(ioDispatcher) {
            val stateBeforeReset = mutableState.value
            val persistedBundle = loadPersistedBundle()
            mobilePushRuntime.unregisterStoredSubscription(
                stateBeforeReset,
                persistedBundle?.ownerNsec,
            )
            if (clearNativeSecretsBeforeReset(showToast = false)) {
                replaceRustCoreAfterReset()
            }
        }
    }

    fun buildSummary(): String = "${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_GIT_SHA})"

    fun relaySetId(): String = BuildConfig.RELAY_SET_ID

    fun isTrustedTestBuild(): Boolean = BuildConfig.TRUSTED_TEST_BUILD

    fun checkForSelfUpdate() {
        selfUpdateManager.check(manual = true)
    }

    fun downloadSelfUpdate() {
        selfUpdateManager.download()
    }

    fun installSelfUpdate(context: Context) {
        selfUpdateManager.install(context)
    }

    fun setSelfUpdateAutoCheckEnabled(enabled: Boolean) {
        selfUpdateManager.setAutoCheckEnabled(enabled)
    }

    private fun applyUpdate(update: AppUpdate) {
        when (update) {
            is AppUpdate.PersistAccountBundle -> {
                // Secure persistence is a shell side effect and must be applied even if snapshot revs race.
                applicationScope.launch(ioDispatcher) {
                    val bundle =
                        StoredAccountBundle(
                            ownerNsec = update.ownerNsec,
                            ownerPubkeyHex = update.ownerPubkeyHex,
                            deviceNsec = update.deviceNsec,
                        )
                    persistBundle(bundle)
                    scheduleMobilePushSyncIfNeeded(
                        mutableState.value,
                        bundle.ownerNsec,
                    )
                }
            }
            is AppUpdate.NearbyPublishedEvent -> {
                publishNearbyEventAsync(update)
            }
            is AppUpdate.FullState -> {
                // Rust owns authoritative state. The shell only accepts the newest full snapshot.
                if (update.v1.rev <= lastRevApplied) {
                    return
                }
                val reconciledState =
                    stateByPreservingVisibleChatPage(
                        oldState = mutableState.value,
                        nextState = stateByReconcilingPendingNavigation(update.v1),
                    )
                lastRevApplied = update.v1.rev
                IrisDebugLog.d(
                    TAG,
                    "reconcile rev=${reconciledState.rev} screen=${reconciledState.router.defaultScreen} " +
                        "chatList=${reconciledState.chatList.size} activeChat=${reconciledState.currentChat?.chatId.orEmpty()} " +
                        "toast=${reconciledState.toast.orEmpty()}",
                )
                publishState(reconciledState)
                scheduleMobilePushSyncIfNeeded(reconciledState, cachedAccountBundle?.ownerNsec)
            }
        }
    }

    private fun publishNearbyEventAsync(update: AppUpdate.NearbyPublishedEvent) {
        val publisher = nearbyEventPublisher ?: return
        val event =
            NearbyPublishedEvent(
                eventId = update.eventId,
                kind = update.kind,
                createdAtSecs = update.createdAtSecs,
                eventJson = update.eventJson,
            )
        applicationScope.launch(ioDispatcher) {
            try {
                nearbyPublishMutex.withLock {
                    publisher(event)
                }
            } catch (error: Throwable) {
                if (isFatalJvmError(error)) {
                    throw error
                }
                logFfiFailure("ffi.update_callback.failed", "NearbyPublishedEvent", error)
                publishShellToast(DISPATCH_FAILURE_TOAST)
            }
        }
    }

    private fun dispatchToRust(
        action: AppAction,
        showsToastOnFailure: Boolean = true,
        preservesPendingNavigation: Boolean = false,
    ): Boolean =
        runCatching {
            if (!preservesPendingNavigation && actionClearsPendingNavigation(action)) {
                pendingNavigationOverride = null
            }
            rust.dispatch(action)
        }.fold(
            onSuccess = { true },
            onFailure = { error ->
                val actionName = actionLogName(action)
                logFfiFailure("ffi.dispatch.failed", actionName, error)
                Log.e(TAG, "FFI dispatch failed ($actionName)", error)
                if (showsToastOnFailure) {
                    publishShellToast(DISPATCH_FAILURE_TOAST)
                }
                false
            },
        )

    private fun stateByReconcilingPendingNavigation(nextState: AppState): AppState {
        val pending = pendingNavigationOverride ?: return nextState
        if (nextState.account == null) {
            pendingNavigationOverride = null
            return nextState
        }
        if (nextState.router.screenStack == pending.stack) {
            pendingNavigationOverride = null
            return nextState
        }
        if (SystemClock.elapsedRealtime() >= pending.expiresAtMs) {
            pendingNavigationOverride = null
            return nextState
        }
        return stateByApplyingLocalScreenStack(pending.stack, nextState)
    }

    private fun stateByPreservingVisibleChatPage(
        oldState: AppState,
        nextState: AppState,
    ): AppState {
        val oldChat = oldState.currentChat ?: return nextState
        val newChat = nextState.currentChat ?: return nextState
        if (
            oldChat.chatId != newChat.chatId ||
                activeChatId(nextState) != newChat.chatId
        ) {
            return nextState
        }
        val newMessageIds = newChat.messages.mapTo(mutableSetOf()) { it.id }
        if (oldChat.messages.all { it.id in newMessageIds }) {
            return nextState
        }
        return nextState.copy(currentChat = mergeChatSnapshot(oldChat, newChat))
    }

    private fun mergeCurrentChatSnapshot(page: CurrentChatSnapshot) {
        val current = mutableState.value
        if (current.currentChat?.chatId != page.chatId) {
            return
        }
        publishState(current.copy(currentChat = mergeChatSnapshot(current.currentChat, page)))
    }

    private fun mergeChatSnapshot(
        existing: CurrentChatSnapshot?,
        page: CurrentChatSnapshot,
    ): CurrentChatSnapshot {
        if (existing?.chatId != page.chatId) {
            return page
        }
        val messagesById = LinkedHashMap<String, ChatMessageSnapshot>(existing.messages.size + page.messages.size)
        existing.messages.forEach { message -> messagesById[message.id] = message }
        page.messages.forEach { message -> messagesById[message.id] = message }
        val mergedMessages = messagesById.values.sortedWith(::compareChatMessages)
        return page.copy(
            participants = page.participants.ifEmpty { existing.participants },
            messages = mergedMessages,
            typingIndicators = page.typingIndicators.ifEmpty { existing.typingIndicators },
            draft = page.draft.ifEmpty { existing.draft },
        )
    }

    private fun compareChatMessages(lhs: ChatMessageSnapshot, rhs: ChatMessageSnapshot): Int {
        lhs.createdAtSecs.compareTo(rhs.createdAtSecs).takeIf { it != 0 }?.let { return it }
        val lhsNumeric = lhs.id.toULongOrNull()
        val rhsNumeric = rhs.id.toULongOrNull()
        if (lhsNumeric != null && rhsNumeric != null && lhsNumeric != rhsNumeric) {
            return lhsNumeric.compareTo(rhsNumeric)
        }
        if (lhsNumeric != null && rhsNumeric == null) {
            return -1
        }
        if (lhsNumeric == null && rhsNumeric != null) {
            return 1
        }
        return lhs.id.compareTo(rhs.id)
    }

    private fun activeChatId(state: AppState): String? =
        when (val active = state.router.screenStack.lastOrNull() ?: state.router.defaultScreen) {
            is Screen.Chat -> active.chatId
            else -> null
        }

    private fun stateByApplyingLocalScreenStack(
        stack: List<Screen>,
        baseState: AppState,
    ): AppState {
        val activeScreen = stack.lastOrNull() ?: baseState.router.defaultScreen
        var currentChat = baseState.currentChat
        var groupDetails = baseState.groupDetails
        when (activeScreen) {
            is Screen.Chat -> {
                if (currentChat?.chatId != activeScreen.chatId) {
                    currentChat = runCatching {
                        rust.chatSnapshot(activeScreen.chatId, CHAT_PAGE_SIZE)
                    }.getOrElse { error ->
                        logFfiFailure(
                            category = "ffi.chat_snapshot.failed",
                            detail = activeScreen.chatId,
                            error = error,
                        )
                        null
                    }
                }
                groupDetails = null
            }
            is Screen.GroupDetails -> {
                if (groupDetails?.groupId != activeScreen.groupId) {
                    groupDetails = null
                }
            }
            is Screen.DirectChatInfo -> {
                if (currentChat?.chatId != activeScreen.chatId) {
                    currentChat = runCatching {
                        rust.chatSnapshot(activeScreen.chatId, CHAT_PAGE_SIZE)
                    }.getOrElse { error ->
                        logFfiFailure(
                            category = "ffi.chat_snapshot.failed",
                            detail = activeScreen.chatId,
                            error = error,
                        )
                        null
                    }
                }
                groupDetails = null
            }
            else -> {
                currentChat = null
                groupDetails = null
            }
        }
        return baseState.copy(
            router = Router(baseState.router.defaultScreen, stack),
            currentChat = currentChat,
            groupDetails = groupDetails,
        )
    }

    private fun actionClearsPendingNavigation(action: AppAction): Boolean =
        when (action) {
            is AppAction.OpenChat,
            is AppAction.PushScreen,
            is AppAction.UpdateScreenStack,
            is AppAction.NavigateBack,
            is AppAction.CreateChat,
            is AppAction.CreateGroup,
            is AppAction.CreateGroupWithPicture,
            is AppAction.AcceptInvite,
            is AppAction.Logout,
            is AppAction.RestoreSession,
            is AppAction.RestoreAccountBundle,
            -> true
            else -> false
        }

    private fun publishShellToast(message: String) {
        val current = mutableState.value
        if (current.toast == message) {
            mutableState.value = current.copy(toast = null)
        }
        mutableState.value = mutableState.value.copy(toast = message)
    }

    private fun actionLogName(action: AppAction): String =
        action::class.simpleName
            ?: action::class.java.simpleName.ifEmpty { "unknown" }

    private fun updateLogName(update: AppUpdate): String =
        update::class.simpleName
            ?: update::class.java.simpleName.ifEmpty { "unknown" }

    private fun isFatalJvmError(error: Throwable): Boolean =
        error is VirtualMachineError || error is ThreadDeath

    private suspend fun restoreSessionFromSecureStore() {
        // Native restore only rehydrates secure inputs. Rust rebuilds the authoritative app state.
        IrisDebugLog.d(TAG, "restoreSessionFromSecureStore start")
        val encrypted = loadPersistedSecret()
        if (encrypted == null) {
            IrisDebugLog.d(TAG, "restoreSessionFromSecureStore no persisted secret")
            restoreCheckComplete = true
            publishBootstrapNeedsLogin()
            return
        }

        val decrypted = runCatching { secureSecretStore.decrypt(encrypted).decodeToString() }.getOrNull()
        if (decrypted.isNullOrBlank()) {
            IrisDebugLog.d(TAG, "restoreSessionFromSecureStore decrypt failed or blank")
            clearPersistedSecret()
            restoreCheckComplete = true
            publishBootstrapNeedsLogin()
            return
        }

        restoreCheckComplete = true
        val bundle = StoredAccountBundle.fromJson(decrypted)
        persistedRestoreInFlight = true
        if (bundle != null) {
            cachedAccountBundle = bundle
            IrisDebugLog.d(TAG, "restoreSessionFromSecureStore dispatch bundle restore")
            val dispatched =
                dispatchToRust(
                    AppAction.RestoreAccountBundle(
                        ownerNsec = bundle.ownerNsec,
                        ownerPubkeyHex = bundle.ownerPubkeyHex,
                        deviceNsec = bundle.deviceNsec,
                    ),
                    showsToastOnFailure = false,
                )
            if (!dispatched) {
                persistedRestoreInFlight = false
                publishBootstrapNeedsLogin()
            }
        } else {
            IrisDebugLog.d(TAG, "restoreSessionFromSecureStore dispatch direct restore")
            val dispatched =
                dispatchToRust(AppAction.RestoreSession(decrypted), showsToastOnFailure = false)
            if (!dispatched) {
                persistedRestoreInFlight = false
                publishBootstrapNeedsLogin()
            }
        }
    }

    private fun bindRust(client: RustAppClient): AppState {
        rust = client
        rustGeneration += 1
        val generation = rustGeneration
        val initial = client.state()
        lastRevApplied = initial.rev
        runCatching {
            client.listenForUpdates(UpdateBridge(generation))
        }.onFailure { error ->
            logFfiFailure("ffi.listen_updates.failed", "generation=$generation", error)
        }
        return initial
    }

    /**
     * Decrypt an incoming FCM push payload against the persisted ratchet
     * state. Returns a resolution with the chat name as the title and
     * the decrypted plaintext as the body. Group notifications use the
     * group name as title and prefix the body with the sender name. If
     * we don't have the secrets yet (logged
     * out, restore in flight) or anything else fails, falls back to the
     * generic resolver so the user still gets *some* notification.
     *
     * Safe to call from the FCM service. Internally this loads the
     * encrypted bundle from the same DataStore + Android Keystore the
     * main process uses, so it works whether the app is alive,
     * background, or just been woken from killed by FCM.
     */
    suspend fun decryptOrResolveNotificationPayload(
        payloadJson: String,
    ): to.iris.chat.rust.MobilePushNotificationResolution {
        dispatchToRust(AppAction.IngestMobilePushPayload(payloadJson), showsToastOnFailure = false)
        val bundle = loadPersistedBundle()
        if (bundle == null) {
            return to.iris.chat.rust.resolveMobilePushNotificationPayload(payloadJson)
        }
        return to.iris.chat.rust.decryptMobilePushNotificationPayload(
            dataDir = rustDataDir,
            ownerPubkeyHex = bundle.ownerPubkeyHex,
            deviceNsec = bundle.deviceNsec,
            rawPayloadJson = payloadJson,
        )
    }

    fun shouldSuppressNotificationForActiveChat(
        resolution: to.iris.chat.rust.MobilePushNotificationResolution,
    ): Boolean {
        if (!appInForeground || !resolution.shouldShow) {
            return false
        }
        val snapshot = mutableState.value
        val activeChatIds = activeNotificationChatIds(snapshot.currentChat, snapshot.router)
        if (activeChatIds.isEmpty()) {
            return false
        }

        val payload = runCatching { JSONObject(resolution.payloadJson) }.getOrNull()
            ?: return false
        return pushNotificationChatCandidates(payload).any { candidate ->
            activeChatIds.any { activeChatId ->
                notificationChatIdMatches(activeChatId, candidate)
            }
        }
    }

    private suspend fun persistBundle(bundle: StoredAccountBundle) {
        val encrypted = secureSecretStore.encrypt(bundle.toJson().encodeToByteArray())
        dataStore.edit { preferences ->
            preferences[SECRET_CIPHERTEXT] = encrypted.cipherText.toBase64()
            preferences[SECRET_IV] = encrypted.iv.toBase64()
        }
        cachedAccountBundle = bundle
    }

    private suspend fun loadPersistedBundle(): StoredAccountBundle? {
        val encrypted = loadPersistedSecret() ?: return null
        val decrypted = runCatching { secureSecretStore.decrypt(encrypted).decodeToString() }.getOrNull()
            ?: return null
        return StoredAccountBundle.fromJson(decrypted)
    }

    private suspend fun loadPersistedSecret(): EncryptedSecret? {
        val preferences =
            dataStore.data
                .catch { throwable ->
                    if (throwable is IOException) {
                        emit(emptyPreferences())
                    } else {
                        throw throwable
                    }
                }.first()

        val cipherText = preferences[SECRET_CIPHERTEXT] ?: return null
        val iv = preferences[SECRET_IV] ?: return null
        return EncryptedSecret(
            cipherText = cipherText.fromBase64(),
            iv = iv.fromBase64(),
        )
    }

    private suspend fun clearPersistedSecret() {
        dataStore.edit { preferences ->
            preferences.remove(SECRET_CIPHERTEXT)
            preferences.remove(SECRET_IV)
        }
        cachedAccountBundle = null
        lastMobilePushSyncInput = null
    }

    private suspend fun clearNativeSecretsBeforeReset(showToast: Boolean = true): Boolean {
        val keyCleared =
            runCatching { secureSecretStore.clear() }
                .onFailure { error -> Log.w(TAG, "failed to clear Android secure key", error) }
                .getOrDefault(false)
        if (!keyCleared) {
            if (showToast) {
                publishShellToast("Could not clear secret key.")
            }
            return false
        }

        val persistedCleared =
            runCatching { clearPersistedSecret() }
                .onFailure { error -> Log.w(TAG, "failed to clear stored account bundle", error) }
                .isSuccess
        val persistedSecretGone =
            runCatching { loadPersistedSecret() == null }
                .onFailure { error -> Log.w(TAG, "failed to verify stored account bundle clear", error) }
                .getOrDefault(false)
        if (!persistedCleared || !persistedSecretGone) {
            if (showToast) {
                publishShellToast("Could not clear secret key.")
            }
            return false
        }
        return true
    }

    private fun scheduleMobilePushSyncIfNeeded(
        state: AppState,
        ownerNsec: String?,
    ) {
        val input = mobilePushSyncInput(state, ownerNsec)
        if (input.ownerPubkeyHex == null) {
            lastMobilePushSyncInput = null
            return
        }
        if (input == lastMobilePushSyncInput) {
            return
        }
        lastMobilePushSyncInput = input
        applicationScope.launch(ioDispatcher) {
            mobilePushRuntime.sync(state, ownerNsec)
        }
    }

    private fun mobilePushSyncInput(
        state: AppState,
        ownerNsec: String?,
    ): AndroidMobilePushSyncInput =
        AndroidMobilePushSyncInput(
            enabled = state.preferences.desktopNotificationsEnabled,
            ownerPubkeyHex = state.mobilePush.ownerPubkeyHex?.trim()?.ifEmpty { null },
            ownerSecretAvailable = !ownerNsec.isNullOrBlank(),
            messageAuthorPubkeys = state.mobilePush.messageAuthorPubkeys,
            inviteResponsePubkeys = state.mobilePush.inviteResponsePubkeys,
            serverOverride =
                state.preferences.mobilePushServerUrl
                    .trim()
                    .ifEmpty { BuildConfig.MOBILE_PUSH_SERVER_URL.trim() },
        )

    private fun replaceRustCoreAfterReset() {
        val previous = rust
        previous.shutdown()
        wipeAppStorage()
        val initial = bindRust(createRustApp())
        restoreCheckComplete = true
        publishState(initial)
    }

    private fun wipeAppStorage() {
        wipeDirectoryContents(appContext.filesDir)
        wipeDirectoryContents(appContext.noBackupFilesDir)
        appContext.getExternalFilesDirs(null).forEach { dir ->
            if (dir != null) {
                wipeDirectoryContents(dir)
            }
        }
        appContext.filesDir.mkdirs()
        appContext.noBackupFilesDir.mkdirs()
    }

    private fun wipeDirectoryContents(directory: File?) {
        val dir = directory ?: return
        if (!dir.exists()) {
            return
        }
        dir.listFiles()?.forEach { child ->
            runCatching { child.deleteRecursively() }
        }
    }

    private fun downloadedAttachmentDirectory(): File =
        File(appContext.cacheDir, "attachments/downloaded").apply { mkdirs() }

    private fun downloadedAttachmentFile(attachment: MessageAttachmentSnapshot): File =
        File(downloadedAttachmentDirectory(), attachmentCacheName(attachment.nhash, attachment.filename))

    private fun pictureCacheFile(nhash: String): File =
        File(downloadedAttachmentDirectory(), "picture-${safeAttachmentCacheComponent(nhash)}")

    private fun cachedDownloadedAttachment(attachment: MessageAttachmentSnapshot): ByteArray? {
        val file = downloadedAttachmentFile(attachment)
        if (!file.isFile) {
            return null
        }
        file.setLastModified(System.currentTimeMillis())
        return runCatching { file.readBytes() }.getOrNull()
    }

    private fun cacheDownloadedAttachment(
        attachment: MessageAttachmentSnapshot,
        data: ByteArray,
    ) {
        val file = downloadedAttachmentFile(attachment)
        runCatching {
            file.writeBytes(data)
            pruneDownloadedAttachmentCache(protectedFile = file)
        }.onFailure { error ->
            Log.w(TAG, "failed to cache attachment", error)
        }
    }

    private fun pruneDownloadedAttachmentCache(protectedFile: File) {
        val files =
            downloadedAttachmentDirectory()
                .listFiles()
                ?.filter { it.isFile }
                ?: return
        var totalSize = files.sumOf { it.length() }
        if (totalSize <= DOWNLOADED_ATTACHMENT_CACHE_LIMIT_BYTES) {
            return
        }

        val protectedPath = protectedFile.canonicalPath
        files
            .sortedBy { it.lastModified() }
            .forEach { file ->
                if (totalSize <= DOWNLOADED_ATTACHMENT_CACHE_LIMIT_BYTES || file.canonicalPath == protectedPath) {
                    return@forEach
                }
                val size = file.length()
                if (file.delete()) {
                    totalSize -= size
                }
            }
    }

    private fun syncCurrentDeviceLabelsIfNeeded(snapshot: AppState) {
        if (snapshot.account == null) {
            lastSyncedDeviceLabelsKey = null
            return
        }
        val currentDevice = snapshot.deviceRoster?.devices?.firstOrNull { it.isCurrentDevice }
        val deviceLabel =
            currentDevice?.deviceLabel?.trim()?.ifEmpty { null } ?: currentPlatformDeviceLabel()
        val clientLabel =
            currentDevice?.clientLabel?.trim()?.ifEmpty { null } ?: "Iris Chat Android"
        val key = "$deviceLabel\u001F$clientLabel"
        if (key == lastSyncedDeviceLabelsKey) {
            return
        }
        lastSyncedDeviceLabelsKey = key
        if (
            !dispatchToRust(
                AppAction.SetCurrentDeviceLabels(deviceLabel, clientLabel),
                showsToastOnFailure = false,
            )
        ) {
            lastSyncedDeviceLabelsKey = null
        }
    }

    private fun currentPlatformDeviceLabel(): String {
        val configuredName =
            runCatching {
                Settings.Global.getString(appContext.contentResolver, Settings.Global.DEVICE_NAME)
            }.getOrNull()?.trim().orEmpty()
        val manufacturer = Build.MANUFACTURER.cleanDeviceLabelPart()
        val model = Build.MODEL.cleanDeviceLabelPart()
        val hardware =
            when {
                model.isEmpty() -> manufacturer
                manufacturer.isEmpty() -> model
                model.lowercase(Locale.ROOT).startsWith(manufacturer.lowercase(Locale.ROOT)) -> model
                else -> "$manufacturer $model"
            }
        return listOfNotNull(
            configuredName.ifEmpty { null },
            hardware.ifEmpty { null },
            "Android ${Build.VERSION.RELEASE}".trim(),
        ).distinctBy { it.lowercase(Locale.ROOT) }.joinToString(" - ")
    }

    private fun String.cleanDeviceLabelPart(): String =
        trim().replace(Regex("\\s+"), " ")

    private fun publishState(snapshot: AppState) {
        IrisDebugLog.enabled = snapshot.preferences.debugLoggingEnabled
        mutableState.value = snapshot
        syncCurrentDeviceLabelsIfNeeded(snapshot)
        if (!restoreCheckComplete) {
            mutableBootstrapState.value = AccountBootstrapState.Loading
            return
        }
        val account = snapshot.account
        if (persistedRestoreInFlight) {
            if (account == null && snapshot.busy.restoringSession) {
                mutableBootstrapState.value = AccountBootstrapState.Loading
                return
            }
            persistedRestoreInFlight = false
        }
        mutableBootstrapState.value =
            when {
                account != null ->
                    AccountBootstrapState.LoggedIn(
                        AccountState(
                            publicKeyHex = account.publicKeyHex,
                            npub = account.npub,
                        ),
                    )
                snapshot.busy.restoringSession -> AccountBootstrapState.Loading
                else -> AccountBootstrapState.NeedsLogin
            }
    }

    private fun publishBootstrapNeedsLogin() {
        restoreCheckComplete = true
        IrisDebugLog.d(TAG, "bootstrap needs login")
        mutableBootstrapState.value = AccountBootstrapState.NeedsLogin
    }

    private fun logFfiFailure(
        category: String,
        detail: String,
        error: Throwable,
    ) {
        val errorDetail =
            buildString {
                if (detail.isNotBlank()) {
                    append(detail)
                    append(": ")
                }
                append(error::class.java.simpleName)
                error.message?.takeIf { it.isNotBlank() }?.let { message ->
                    append(": ")
                    append(message)
                }
            }
        appendClientDebugLog(category, errorDetail)
        IrisDebugLog.d(TAG, "$category $detail", error)
    }

    private fun appendClientDebugLog(category: String, detail: String) {
        val entry =
            ClientDebugLogEntry(
                timestampSecs = currentTimeSeconds(),
                category = category,
                detail = detail.take(MAX_CLIENT_DEBUG_LOG_DETAIL_CHARS),
            )
        synchronized(clientDebugLog) {
            clientDebugLog.add(entry)
            while (clientDebugLog.size > MAX_CLIENT_DEBUG_LOG_ENTRIES) {
                clientDebugLog.removeAt(0)
            }
        }
    }

    private fun snapshotClientDebugLog(): List<ClientDebugLogEntry> =
        synchronized(clientDebugLog) {
            clientDebugLog.toList()
        }

    private fun supportBundleJsonWithClientLog(
        rustJson: String,
        clientLog: List<ClientDebugLogEntry>,
    ): String {
        if (clientLog.isEmpty()) {
            return rustJson
        }
        return runCatching {
            val root = JSONObject(rustJson.ifBlank { "{}" })
            val array = JSONArray()
            clientLog.forEach { array.put(it.toJsonObject()) }
            root.put("client_log", array)
            root.toString(2)
        }.getOrDefault(rustJson)
    }

    private fun ByteArray.toBase64(): String = Base64.encodeToString(this, Base64.NO_WRAP)

    private fun String.fromBase64(): ByteArray = Base64.decode(this, Base64.NO_WRAP)

    private companion object {
        const val TAG = "NdrDebug"
        const val DATASTORE_NAME = "iris_chat_secure_store.preferences_pb"
        const val DISPATCH_FAILURE_TOAST = "Action failed. Copy support bundle in Settings."
        const val NAVIGATION_OVERRIDE_TTL_MS = 10_000L
        const val CHAT_PAGE_SIZE = 80u
        const val CHAT_AROUND_BEFORE_LIMIT = 40u
        const val CHAT_AROUND_AFTER_LIMIT = 40u
        const val DOWNLOADED_ATTACHMENT_CACHE_LIMIT_BYTES = 128L * 1024L * 1024L
        const val MAX_CLIENT_DEBUG_LOG_ENTRIES = 50
        const val MAX_CLIENT_DEBUG_LOG_DETAIL_CHARS = 1_000
        val SECRET_CIPHERTEXT = stringPreferencesKey("secret_ciphertext")
        val SECRET_IV = stringPreferencesKey("secret_iv")

        fun attachmentCacheName(
            nhash: String,
            filename: String,
        ): String = "${safeAttachmentCacheComponent(nhash)}-${safeAttachmentCacheComponent(filename)}"

        fun safeAttachmentCacheComponent(value: String): String =
            value
                .split('/', '\\', ':')
                .joinToString("-")
                .trim()
                .ifEmpty { "attachment" }

        fun appVersion(context: Context): String =
            runCatching {
                context.packageManager.getPackageInfo(context.packageName, 0).versionName
            }.getOrNull()
                ?: "0.1.0"

        fun currentTimeSeconds(): Long =
            System.currentTimeMillis() / 1_000L
    }

    private fun createRustApp(): RustAppClient =
        rustFactory?.invoke(rustDataDir, appVersion(appContext))
            ?: LiveRustAppClient(
                dataDir = rustDataDir,
                appVersion = appVersion(appContext),
            )

    private inner class UpdateBridge(
        private val generation: Long,
    ) : AppReconciler {
        override fun reconcile(update: AppUpdate) {
            if (generation != rustGeneration) {
                return
            }
            try {
                applyUpdate(update)
            } catch (error: Throwable) {
                if (isFatalJvmError(error)) {
                    throw error
                }
                val updateName = updateLogName(update)
                logFfiFailure("ffi.update_callback.failed", updateName, error)
                Log.e(TAG, "FFI update callback failed ($updateName)", error)
                publishShellToast(DISPATCH_FAILURE_TOAST)
            }
        }
    }
}
