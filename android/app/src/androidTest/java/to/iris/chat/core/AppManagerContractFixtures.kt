package to.iris.chat.core

import to.iris.chat.account.EncryptedSecret
import to.iris.chat.account.SecureSecretStore
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AppState
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.buildLargeTestSearchResult

internal class RecordingSecureSecretStore : SecureSecretStore {
    var clearCount = 0
    var clearSucceeds = true

    override fun encrypt(secret: ByteArray): EncryptedSecret =
        EncryptedSecret(cipherText = secret, iv = byteArrayOf(1, 2, 3, 4))

    override fun decrypt(encryptedSecret: EncryptedSecret): ByteArray = encryptedSecret.cipherText

    override fun clear(): Boolean {
        clearCount += 1
        return clearSucceeds
    }
}

internal class RecordingRustFactory {
    val initialStates = ArrayDeque<AppState>()
    val instances = mutableListOf<MockRustAppClient>()

    fun create(): RustAppClient {
        val initialState = initialStates.removeFirstOrNull() ?: AppManagerContractDefaults.initialState()
        return MockRustAppClient(initialState).also(instances::add)
    }
}

internal class MockRustAppClient(
    var currentState: AppState,
) : RustAppClient {
    val dispatchedActions = mutableListOf<AppAction>()
    var peerDebug: PeerProfileDebugSnapshot? = null
    var dispatchError: Throwable? = null
    var prepareForSuspendCount = 0
    var shutdownCount = 0
    val pagesBefore = mutableMapOf<Pair<String, String>, CurrentChatSnapshot>()
    val pagesAround = mutableMapOf<Pair<String, String>, CurrentChatSnapshot>()
    private var reconciler: AppReconciler? = null

    override fun state(): AppState = currentState

    override fun dispatch(action: AppAction) {
        dispatchError?.let { throw it }
        dispatchedActions += action
    }

    override fun acceptDirectFiles(chatId: String, transferId: String, destination: to.iris.chat.rust.DirectFileDestination) {
        dispatchError?.let { throw it }
    }

    override fun search(query: String, scopeChatId: String?, limit: UInt): SearchResultSnapshot {
        val messageCount = if (limit > 120u) limit else 120u
        return buildLargeTestSearchResult(
            query = query,
            personCount = 11u,
            contactCount = 25u,
            groupCount = 9u,
            messageCount = messageCount,
        ).also { result ->
            result.scopeChatId = scopeChatId
            if (scopeChatId != null) {
                result.people = emptyList()
                result.contacts = emptyList()
                result.groups = emptyList()
            }
        }
    }

    override fun mutualGroups(ownerInput: String): List<ChatThreadSnapshot> = emptyList()

    override fun chatSnapshot(chatId: String, limit: UInt): CurrentChatSnapshot? {
        val trimmed = chatId.trim()
        if (trimmed.isEmpty() || currentState.account == null) {
            return null
        }
        currentState.currentChat?.takeIf { it.chatId == trimmed }?.let { return it }
        val thread = currentState.chatList.firstOrNull { it.chatId == trimmed }
        val groupId = trimmed.removePrefix("group:").takeIf { trimmed.startsWith("group:") }
        return CurrentChatSnapshot(contactIdentity = null, socialConnection = null, chatId = trimmed,
            kind = thread?.kind ?: if (groupId == null) ChatKind.DIRECT else ChatKind.GROUP,
            displayName = thread?.displayName ?: trimmed,
            nickname = thread?.nickname,
            contactNote = thread?.contactNote,
            profileName = thread?.profileName,
            subtitle = thread?.subtitle,
            pictureUrl = thread?.pictureUrl,
            about = thread?.about,
            groupId = groupId,
            memberCount = thread?.memberCount ?: 0u,
            messageTtlSeconds = null,
            isMuted = thread?.isMuted ?: false,
            participants = emptyList(),
            messages = emptyList(),
            typingIndicators = emptyList(),
            draft = thread?.draft.orEmpty(),
            isRequest = thread?.isRequest ?: false, directChatCapability = if (groupId == null) to.iris.chat.rust.DirectChatCapabilityState.AVAILABLE else null,
        )
    }

    override fun chatSnapshotBefore(chatId: String, beforeMessageId: String, limit: UInt): CurrentChatSnapshot? =
        pagesBefore[chatId.trim() to beforeMessageId.trim()]

    override fun chatSnapshotAroundMessage(
        chatId: String,
        messageId: String,
        beforeLimit: UInt,
        afterLimit: UInt,
    ): CurrentChatSnapshot? =
        pagesAround[chatId.trim() to messageId.trim()]

    override fun exportSupportBundleJson(): String = """{"ok":true}"""

    override fun peerProfileDebug(ownerInput: String): PeerProfileDebugSnapshot? = peerDebug

    override fun prepareForSuspend() {
        prepareForSuspendCount += 1
    }

    override fun listenForUpdates(reconciler: AppReconciler) {
        this.reconciler = reconciler
    }

    override fun shutdown() {
        shutdownCount += 1
    }

    fun emit(update: AppUpdate) {
        reconciler?.reconcile(update)
    }
}
