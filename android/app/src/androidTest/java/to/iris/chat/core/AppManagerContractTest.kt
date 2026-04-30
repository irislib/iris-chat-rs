package to.iris.chat.core

import android.content.Context
import android.os.SystemClock
import android.util.Base64
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStoreFile
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.account.EncryptedSecret
import to.iris.chat.account.SecureSecretStore
import to.iris.chat.account.StoredAccountBundle
import to.iris.chat.rust.AccountSnapshot
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AppState
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.BusyState
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.MobilePushSyncSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.Router
import to.iris.chat.rust.Screen

@RunWith(AndroidJUnit4::class)
class AppManagerContractTest {
    private lateinit var appContext: Context
    private lateinit var applicationScope: CoroutineScope
    private lateinit var secureSecretStore: RecordingSecureSecretStore
    private lateinit var rustFactory: RecordingRustFactory
    private lateinit var dataStoreName: String
    private lateinit var sharedDataStore: DataStore<Preferences>
    private var manager: AppManager? = null

    @Before
    fun setUp() {
        appContext = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        secureSecretStore = RecordingSecureSecretStore()
        rustFactory = RecordingRustFactory()
        dataStoreName = "app-manager-contract-${UUID.randomUUID()}.preferences_pb"
        sharedDataStore =
            PreferenceDataStoreFactory.create(
                scope = applicationScope,
                produceFile = { appContext.preferencesDataStoreFile(dataStoreName) },
            )
    }

    @After
    fun tearDown() {
        manager?.resetForUiTestsBlocking()
        manager = null
        applicationScope.cancel()
        runCatching { appContext.preferencesDataStoreFile(dataStoreName).delete() }
    }

    @Test
    fun startup_without_stored_credentials_settles_to_needs_login() {
        val appManager = createManager()

        waitFor("bootstrap settles without credentials") {
            appManager.bootstrapState.value is AccountBootstrapState.NeedsLogin
        }

        assertTrue(rustFactory.instances.single().dispatchedActions.isEmpty())
    }

    @Test
    fun foreground_dispatches_relay_refresh_action() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.appForegrounded()

        assertTrue(rust.dispatchedActions.contains(AppAction.AppForegrounded))
    }

    @Test
    fun create_group_allows_empty_member_list() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.createGroup("  Notes  ", emptyList())

        val action = rust.dispatchedActions.single()
        assertTrue(action is AppAction.CreateGroup)
        action as AppAction.CreateGroup
        assertEquals("Notes", action.name)
        assertEquals(emptyList<String>(), action.memberInputs)
    }

    @Test
    fun dispatch_failure_shows_diagnostic_toast_instead_of_throwing() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        rust.dispatchError = IllegalStateException("ffi failed")

        appManager.dispatch(AppAction.PushScreen(Screen.NewChat))

        assertEquals("Action failed. Copy support bundle in Settings.", appManager.state.value.toast)
        assertTrue(rust.dispatchedActions.isEmpty())
    }

    @Test
    fun update_side_effect_failure_does_not_escape_reconciler_callback() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        appManager.setNearbyEventPublisher {
            throw LinkageError("bluetooth callback failed")
        }

        rust.emit(
            AppUpdate.NearbyPublishedEvent(
                eventId = "a".repeat(64),
                kind = 1u,
                createdAtSecs = 42u,
                eventJson = """{"id":"${"a".repeat(64)}"}""",
            ),
        )

        assertEquals("Action failed. Copy support bundle in Settings.", appManager.state.value.toast)
    }

    @Test
    fun restore_from_stored_bundle_dispatches_restore_account_bundle() {
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )

        createManager()
        val firstRust = rustFactory.instances.single()

        waitFor("restore account bundle dispatch") {
            firstRust.dispatchedActions.isNotEmpty()
        }

        val action = firstRust.dispatchedActions.single()
        assertTrue(action is AppAction.RestoreAccountBundle)
        action as AppAction.RestoreAccountBundle
        assertEquals("nsec1owner", action.ownerNsec)
        assertEquals("owner-hex", action.ownerPubkeyHex)
        assertEquals("nsec1device", action.deviceNsec)
    }

    @Test
    fun legacy_direct_secret_restore_dispatches_restore_session() {
        persistStoredSecret("nsec1legacy")

        createManager()
        val firstRust = rustFactory.instances.single()

        waitFor("legacy restore dispatch") {
            firstRust.dispatchedActions.isNotEmpty()
        }

        val action = firstRust.dispatchedActions.single()
        assertTrue(action is AppAction.RestoreSession)
        action as AppAction.RestoreSession
        assertEquals("nsec1legacy", action.ownerNsec)
    }

    @Test
    fun stale_full_state_updates_are_dropped() {
        rustFactory.initialStates += makeAppState(rev = 1u)
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        val newer = makeAppState(rev = 2u, router = Router(Screen.ChatList, emptyList()), toast = "synced")
        val older = makeAppState(rev = 1u, toast = "stale")

        rust.emit(AppUpdate.FullState(newer))
        waitFor("newer snapshot applied") {
            appManager.state.value.rev == 2uL
        }
        rust.emit(AppUpdate.FullState(older))
        SystemClock.sleep(100)

        assertEquals(2uL, appManager.state.value.rev)
        assertEquals("synced", appManager.state.value.toast)
    }

    @Test
    fun persist_account_bundle_side_effect_applies_even_when_stale() {
        rustFactory.initialStates += makeAppState(rev = 5u)
        createManager()
        val rust = rustFactory.instances.single()

        rust.emit(
            AppUpdate.PersistAccountBundle(
                rev = 1u,
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ),
        )

        waitFor("persisted account bundle") {
            loadPersistedBundle() != null
        }

        assertEquals(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ),
            loadPersistedBundle(),
        )
    }

    @Test
    fun export_secret_keys_reads_persisted_account_bundle() {
        val appManager = createManager()
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )

        val ownerNsec = runBlocking { appManager.exportOwnerNsec() }
        val deviceNsec = runBlocking { appManager.exportDeviceNsec() }

        assertEquals("nsec1owner", ownerNsec)
        assertEquals("nsec1device", deviceNsec)
    }

    @Test
    fun export_owner_secret_returns_null_for_linked_device_bundle() {
        val appManager = createManager()
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = null,
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )

        val ownerNsec = runBlocking { appManager.exportOwnerNsec() }
        val deviceNsec = runBlocking { appManager.exportDeviceNsec() }

        assertNull(ownerNsec)
        assertEquals("nsec1device", deviceNsec)
    }

    @Test
    fun logout_clears_native_secrets_and_app_files_then_rebinds_fresh_rust_core() {
        rustFactory.initialStates += makeLoggedInState(rev = 5u)
        rustFactory.initialStates += makeAppState(rev = 0u)
        val appManager = createManager()
        val firstRust = rustFactory.instances.single()
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )
        val staleFile = appContext.filesDir.resolve("contract-logout-${UUID.randomUUID()}.txt")
        staleFile.writeText("stale")

        appManager.logout()

        waitFor("fresh rust core after logout") {
            rustFactory.instances.size == 2
        }
        val secondRust = rustFactory.instances[1]

        assertTrue(firstRust.dispatchedActions.contains(AppAction.Logout))
        assertEquals(1, firstRust.shutdownCount)
        assertEquals(1, secureSecretStore.clearCount)
        assertNull(loadPersistedBundle())
        assertFalse(staleFile.exists())
        assertNull(appManager.state.value.account)
        assertEquals(secondRust.currentState, appManager.state.value)
        assertTrue(appManager.bootstrapState.value is AccountBootstrapState.NeedsLogin)
    }

    @Test
    fun reset_for_ui_tests_rebinds_fresh_rust_core_and_clears_shell_state() {
        rustFactory.initialStates += makeLoggedInState(rev = 3u)
        rustFactory.initialStates += makeAppState(rev = 0u)
        val appManager = createManager()
        val firstRust = rustFactory.instances.single()
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )
        val staleFile = appContext.filesDir.resolve("contract-reset-${UUID.randomUUID()}.txt")
        staleFile.writeText("stale")

        appManager.resetForUiTestsBlocking()

        assertEquals(2, rustFactory.instances.size)
        val secondRust = rustFactory.instances[1]
        assertEquals(1, firstRust.shutdownCount)
        assertEquals(1, secureSecretStore.clearCount)
        assertNull(loadPersistedBundle())
        assertFalse(staleFile.exists())
        assertNull(appManager.state.value.account)
        assertEquals(secondRust.currentState, appManager.state.value)
        assertTrue(appManager.bootstrapState.value is AccountBootstrapState.NeedsLogin)
    }

    private fun createManager(): AppManager {
        val appManager =
            AppManager(
                context = appContext,
                applicationScope = applicationScope,
                secureSecretStore = secureSecretStore,
                ioDispatcher = Dispatchers.IO,
                dataStoreName = dataStoreName,
                dataStore = sharedDataStore,
                rustFactory = { _, _ -> rustFactory.create() },
            )
        manager = appManager
        return appManager
    }

    private fun persistStoredSecret(value: String) {
        val encrypted = secureSecretStore.encrypt(value.encodeToByteArray())
        runBlocking {
            sharedDataStore.edit { preferences ->
                preferences[SECRET_CIPHERTEXT] = Base64.encodeToString(encrypted.cipherText, Base64.NO_WRAP)
                preferences[SECRET_IV] = Base64.encodeToString(encrypted.iv, Base64.NO_WRAP)
            }
        }
    }

    private fun loadPersistedBundle(): StoredAccountBundle? {
        val encrypted =
            runBlocking {
                val preferences =
                    sharedDataStore.data
                        .catch { throwable ->
                            if (throwable is IOException) {
                                emit(emptyPreferences())
                            } else {
                                throw throwable
                            }
                        }.first()
                val cipherText = preferences[SECRET_CIPHERTEXT] ?: return@runBlocking null
                val iv = preferences[SECRET_IV] ?: return@runBlocking null
                EncryptedSecret(
                    cipherText = Base64.decode(cipherText, Base64.NO_WRAP),
                    iv = Base64.decode(iv, Base64.NO_WRAP),
                )
            } ?: return null

        val raw = secureSecretStore.decrypt(encrypted).decodeToString()
        return StoredAccountBundle.fromJson(raw)
    }

    private fun waitFor(
        description: String,
        timeoutMs: Long = 5_000,
        predicate: () -> Boolean,
    ) {
        val deadline = SystemClock.elapsedRealtime() + timeoutMs
        while (SystemClock.elapsedRealtime() < deadline) {
            if (predicate()) {
                return
            }
            SystemClock.sleep(25)
        }
        throw AssertionError("Timed out waiting for $description")
    }

    private fun makeAppState(
        rev: ULong = 0u,
        router: Router = Router(Screen.Welcome, emptyList()),
        toast: String? = null,
        account: AccountSnapshot? = null,
    ): AppState =
        AppState(
            rev = rev,
            router = router,
            account = account,
            deviceRoster = null,
            busy =
                BusyState(
                    creatingAccount = false,
                    restoringSession = false,
                    linkingDevice = false,
                    creatingChat = false,
                    creatingGroup = false,
                    sendingMessage = false,
                    updatingRoster = false,
                    updatingGroup = false,
                    creatingInvite = false,
                    acceptingInvite = false,
                    syncingNetwork = false,
                    uploadingAttachment = false,
                ),
            chatList = emptyList(),
            currentChat = null,
            groupDetails = null,
            publicInvite = null,
            linkDevice = null,
            networkStatus = null,
            mobilePush = MobilePushSyncSnapshot(null, emptyList(), emptyList(), emptyList()),
            preferences =
                PreferencesSnapshot(
                    sendTypingIndicators = true,
                    sendReadReceipts = true,
                    desktopNotificationsEnabled = true,
                    inviteAcceptanceNotificationsEnabled = true,
                    startupAtLoginEnabled = false,
                    nearbyBluetoothEnabled = false,
                    nearbyLanEnabled = false,
                    nostrRelayUrls =
                        listOf(
                            "wss://relay.damus.io",
                            "wss://nos.lol",
                            "wss://relay.primal.net",
                            "wss://relay.snort.social",
                            "wss://temp.iris.to",
                        ),
                    imageProxyEnabled = true,
                    imageProxyUrl = "https://imgproxy.iris.to",
                    imageProxyKeyHex = "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                    imageProxySaltHex = "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
                    mutedChatIds = emptyList(),
                    mobilePushServerUrl = "",
                ),
            toast = toast,
        )

    private fun makeLoggedInState(rev: ULong): AppState =
        makeAppState(
            rev = rev,
            router = Router(Screen.ChatList, emptyList()),
            account =
                AccountSnapshot(
                    publicKeyHex = "owner-hex",
                    npub = "npub1owner",
                    displayName = "Owner",
                    pictureUrl = null,
                    devicePublicKeyHex = "device-hex",
                    deviceNpub = "npub1device",
                    hasOwnerSigningAuthority = true,
                    authorizationState = DeviceAuthorizationState.AUTHORIZED,
                ),
        )

    private companion object {
        val SECRET_CIPHERTEXT = stringPreferencesKey("secret_ciphertext")
        val SECRET_IV = stringPreferencesKey("secret_iv")
    }
}

private class RecordingSecureSecretStore : SecureSecretStore {
    var clearCount = 0

    override fun encrypt(secret: ByteArray): EncryptedSecret =
        EncryptedSecret(cipherText = secret, iv = byteArrayOf(1, 2, 3, 4))

    override fun decrypt(encryptedSecret: EncryptedSecret): ByteArray = encryptedSecret.cipherText

    override fun clear() {
        clearCount += 1
    }
}

private class RecordingRustFactory {
    val initialStates = ArrayDeque<AppState>()
    val instances = mutableListOf<MockRustAppClient>()

    fun create(): RustAppClient {
        val initialState = initialStates.removeFirstOrNull() ?: AppManagerContractDefaults.initialState()
        return MockRustAppClient(initialState).also(instances::add)
    }
}

private class MockRustAppClient(
    var currentState: AppState,
) : RustAppClient {
    val dispatchedActions = mutableListOf<AppAction>()
    var dispatchError: Throwable? = null
    var shutdownCount = 0
    private var reconciler: AppReconciler? = null

    override fun state(): AppState = currentState

    override fun dispatch(action: AppAction) {
        dispatchError?.let { throw it }
        dispatchedActions += action
    }

    override fun ingestNearbyEventJson(eventJson: String): Boolean = true

    override fun buildNearbyPresenceEventJson(
        peerId: String,
        myNonce: String,
        theirNonce: String,
        profileEventId: String,
    ): String = ""

    override fun verifyNearbyPresenceEventJson(
        eventJson: String,
        peerId: String,
        myNonce: String,
        theirNonce: String,
    ): String = ""

    override fun nearbyEncodeFrame(envelopeJson: String): ByteArray = ByteArray(0)

    override fun nearbyDecodeFrame(frame: ByteArray): String = ""

    override fun nearbyFrameBodyLenFromHeader(header: ByteArray): Int = -1

    override fun exportSupportBundleJson(): String = """{"ok":true}"""

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

private object AppManagerContractDefaults {
    fun initialState(): AppState =
        AppState(
            rev = 0u,
            router = Router(Screen.Welcome, emptyList()),
            account = null,
            deviceRoster = null,
            busy =
                BusyState(
                    creatingAccount = false,
                    restoringSession = false,
                    linkingDevice = false,
                    creatingChat = false,
                    creatingGroup = false,
                    sendingMessage = false,
                    updatingRoster = false,
                    updatingGroup = false,
                    creatingInvite = false,
                    acceptingInvite = false,
                    syncingNetwork = false,
                    uploadingAttachment = false,
                ),
            chatList = emptyList(),
            currentChat = null,
            groupDetails = null,
            publicInvite = null,
            linkDevice = null,
            networkStatus = null,
            mobilePush = MobilePushSyncSnapshot(null, emptyList(), emptyList(), emptyList()),
            preferences =
                PreferencesSnapshot(
                    sendTypingIndicators = true,
                    sendReadReceipts = true,
                    desktopNotificationsEnabled = true,
                    inviteAcceptanceNotificationsEnabled = true,
                    startupAtLoginEnabled = false,
                    nearbyBluetoothEnabled = false,
                    nearbyLanEnabled = false,
                    nostrRelayUrls =
                        listOf(
                            "wss://relay.damus.io",
                            "wss://nos.lol",
                            "wss://relay.primal.net",
                            "wss://relay.snort.social",
                            "wss://temp.iris.to",
                        ),
                    imageProxyEnabled = true,
                    imageProxyUrl = "https://imgproxy.iris.to",
                    imageProxyKeyHex = "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
                    imageProxySaltHex = "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
                    mutedChatIds = emptyList(),
                    mobilePushServerUrl = "",
                ),
            toast = null,
        )
}
