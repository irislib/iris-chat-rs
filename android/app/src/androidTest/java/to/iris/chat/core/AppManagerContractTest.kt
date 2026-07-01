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
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
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
import to.iris.chat.rust.ChatKind
import to.iris.chat.rust.ChatMessageKind
import to.iris.chat.rust.ChatMessageSnapshot
import to.iris.chat.rust.ChatThreadSnapshot
import to.iris.chat.rust.CurrentChatSnapshot
import to.iris.chat.rust.DeviceAuthorizationState
import to.iris.chat.rust.DeliveryState
import to.iris.chat.rust.MessageDeliveryTraceSnapshot
import to.iris.chat.rust.MobilePushNotificationResolution
import to.iris.chat.rust.MobilePushSyncSnapshot
import to.iris.chat.rust.PeerProfileDebugSnapshot
import to.iris.chat.rust.PreferencesSnapshot
import to.iris.chat.rust.Router
import to.iris.chat.rust.SearchResultSnapshot
import to.iris.chat.rust.Screen
import to.iris.chat.rust.buildLargeTestAppState
import to.iris.chat.rust.buildLargeTestSearchResult

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
    fun background_flushes_rust_core_before_process_can_be_stopped() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.appBackgrounded()

        assertEquals(1, rust.prepareForSuspendCount)
    }

    @Test
    fun active_chat_notification_suppression_matches_direct_sender() {
        rustFactory.initialStates += makeLargeFixtureState(
            currentChat = makeCurrentChat(chatId = "ABCDEF", kind = ChatKind.DIRECT),
        )
        val appManager = createManager()
        appManager.appForegrounded()

        val resolution =
            MobilePushNotificationResolution(
                shouldShow = true,
                title = "Alice",
                body = "hello",
                payloadJson = """{"sender_pubkey":"abcdef"}""",
            )

        assertTrue(appManager.shouldSuppressNotificationForActiveChat(resolution))
    }

    @Test
    fun active_chat_notification_suppression_matches_group_payload_id() {
        rustFactory.initialStates += makeLargeFixtureState(
            currentChat = makeCurrentChat(
                chatId = "group:Group-123",
                kind = ChatKind.GROUP,
                groupId = "Group-123",
            ),
        )
        val appManager = createManager()
        appManager.appForegrounded()

        val resolution =
            MobilePushNotificationResolution(
                shouldShow = true,
                title = "Group",
                body = "hello",
                payloadJson = """{"group_id":"group-123"}""",
            )

        assertTrue(appManager.shouldSuppressNotificationForActiveChat(resolution))
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

        waitFor("nearby failure toast") {
            appManager.state.value.toast == "Action failed. Copy support bundle in Settings."
        }
    }

    @Test
    fun nearby_publish_does_not_block_reconciler_callback() {
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        val publisherEntered = CountDownLatch(1)
        val releasePublisher = CountDownLatch(1)
        appManager.setNearbyEventPublisher {
            publisherEntered.countDown()
            releasePublisher.await(5, TimeUnit.SECONDS)
        }

        val callbackReturned = CountDownLatch(1)
        val callbackThread =
            Thread {
                rust.emit(
                    AppUpdate.NearbyPublishedEvent(
                        eventId = "b".repeat(64),
                        kind = 14u,
                        createdAtSecs = 43u,
                        eventJson = """{"id":"${"b".repeat(64)}"}""",
                    ),
                )
                callbackReturned.countDown()
            }

        callbackThread.start()
        try {
            assertTrue("nearby publisher did not start", publisherEntered.await(1, TimeUnit.SECONDS))
            assertTrue(
                "nearby publish blocked the reconciler callback",
                callbackReturned.await(250, TimeUnit.MILLISECONDS),
            )
        } finally {
            releasePublisher.countDown()
            callbackThread.join(1_000)
        }
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

        val appManager = createManager()
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
        assertTrue(appManager.bootstrapState.value is AccountBootstrapState.Loading)

        firstRust.emit(AppUpdate.FullState(makeLoggedInState(rev = 1u)))
        waitFor("bootstrap settles after restored account") {
            appManager.bootstrapState.value is AccountBootstrapState.LoggedIn
        }
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
        rustFactory.initialStates += makeLargeFixtureState(rev = 1u)
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        val newer =
            makeLargeFixtureState(
                rev = 2u,
                router = Router(Screen.ChatList, emptyList()),
                toast = "synced",
            )
        val older = makeLargeFixtureState(rev = 1u, toast = "stale")

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
    fun navigate_back_dispatches_explicit_stack_and_updates_shell_immediately() {
        val chatScreen = Screen.Chat("chat-1")
        val initial = makeLoggedInState(rev = 1u).also { state ->
            state.router = Router(Screen.ChatList, listOf(chatScreen))
            state.currentChat = makeCurrentChat(chatId = "chat-1", kind = ChatKind.DIRECT)
        }
        rustFactory.initialStates += initial
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.navigateBack()

        val updateAction = rust.dispatchedActions.filterIsInstance<AppAction.UpdateScreenStack>().single()
        assertEquals(emptyList<Screen>(), updateAction.stack)
        assertEquals(emptyList<Screen>(), appManager.state.value.router.screenStack)
        assertNull(appManager.state.value.currentChat)
    }

    @Test
    fun navigate_back_keeps_local_route_while_rust_catches_up() {
        val chatScreen = Screen.Chat("chat-1")
        val initial = makeLoggedInState(rev = 1u).also { state ->
            state.router = Router(Screen.ChatList, listOf(chatScreen))
            state.currentChat = makeCurrentChat(chatId = "chat-1", kind = ChatKind.DIRECT)
        }
        rustFactory.initialStates += initial
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.navigateBack()
        rust.emit(
            AppUpdate.FullState(
                makeLoggedInState(rev = 2u).also { state ->
                    state.router = Router(Screen.ChatList, listOf(chatScreen))
                    state.currentChat = makeCurrentChat(chatId = "chat-1", kind = ChatKind.DIRECT)
                },
            ),
        )
        waitFor("stale route reconciled with local back") {
            appManager.state.value.rev == 2uL
        }

        assertEquals(emptyList<Screen>(), appManager.state.value.router.screenStack)
        assertNull(appManager.state.value.currentChat)

        rust.emit(
            AppUpdate.FullState(
                makeLoggedInState(rev = 3u).also { state ->
                    state.router = Router(Screen.ChatList, emptyList())
                    state.currentChat = null
                },
            ),
        )
        waitFor("rust catches up to local back") {
            appManager.state.value.rev == 3uL
        }

        assertEquals(emptyList<Screen>(), appManager.state.value.router.screenStack)
        assertNull(appManager.state.value.currentChat)
    }

    @Test
    fun open_chat_updates_shell_route_before_rust_catches_up() {
        val initial = makeLoggedInState(rev = 1u)
        rustFactory.initialStates += initial
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.dispatch(AppAction.OpenChat("chat-1"))

        assertTrue(rust.dispatchedActions.contains(AppAction.OpenChat("chat-1")))
        assertEquals(listOf(Screen.Chat("chat-1")), appManager.state.value.router.screenStack)
        assertEquals("chat-1", appManager.state.value.currentChat?.chatId)

        rust.emit(
            AppUpdate.FullState(
                makeLoggedInState(rev = 2u).also { state ->
                    state.router = Router(Screen.ChatList, emptyList())
                },
            ),
        )
        waitFor("stale route reconciled with local open") {
            appManager.state.value.rev == 2uL
        }

        assertEquals(listOf(Screen.Chat("chat-1")), appManager.state.value.router.screenStack)
    }

    @Test
    fun open_chat_at_message_loads_search_hit_page_outside_initial_page() {
        rustFactory.initialStates += makeLoggedInState(rev = 1u)
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        val aroundPage =
            makeCurrentChat(
                chatId = "chat-1",
                kind = ChatKind.DIRECT,
                messages = (15..35).map { makeMessage("chat-1", it.toString()) },
            )
        rust.pagesAround["chat-1" to "25"] = aroundPage

        appManager.openChatAtMessage("chat-1", "25")

        assertTrue(rust.dispatchedActions.contains(AppAction.OpenChat("chat-1")))
        assertEquals(listOf(Screen.Chat("chat-1")), appManager.state.value.router.screenStack)
        waitFor("search hit page merge") {
            appManager.state.value.currentChat?.messages.orEmpty().any { it.id == "25" }
        }
        assertTrue(appManager.state.value.currentChat?.messages.orEmpty().any { it.id == "25" })
    }

    @Test
    fun full_state_keeps_loaded_search_hit_context_for_visible_chat() {
        val initial =
            makeLoggedInState(rev = 1u).also { state ->
                state.router = Router(Screen.ChatList, listOf(Screen.Chat("chat-1")))
                state.currentChat =
                    makeCurrentChat(
                        chatId = "chat-1",
                        kind = ChatKind.DIRECT,
                        messages = (15..35).map { makeMessage("chat-1", it.toString()) },
                    )
            }
        rustFactory.initialStates += initial
        val appManager = createManager()
        val rust = rustFactory.instances.single()
        val latestPage =
            makeCurrentChat(
                chatId = "chat-1",
                kind = ChatKind.DIRECT,
                messages = (121..200).map { makeMessage("chat-1", it.toString()) },
            )

        rust.emit(
            AppUpdate.FullState(
                makeLoggedInState(rev = 2u).also { state ->
                    state.router = Router(Screen.ChatList, listOf(Screen.Chat("chat-1")))
                    state.currentChat = latestPage
                },
            ),
        )
        waitFor("visible chat page preserved") {
            appManager.state.value.rev == 2uL
        }

        val messages = appManager.state.value.currentChat?.messages.orEmpty()
        assertTrue(messages.any { it.id == "25" })
        assertTrue(messages.any { it.id == "200" })
        assertEquals("15", messages.first().id)
        assertEquals("200", messages.last().id)
    }

    @Test
    fun push_screen_updates_shell_route_before_rust_catches_up() {
        val initial = makeLoggedInState(rev = 1u)
        rustFactory.initialStates += initial
        val appManager = createManager()
        val rust = rustFactory.instances.single()

        appManager.dispatch(AppAction.PushScreen(Screen.Settings))

        assertTrue(rust.dispatchedActions.contains(AppAction.PushScreen(Screen.Settings)))
        assertEquals(listOf(Screen.Settings), appManager.state.value.router.screenStack)

        rust.emit(
            AppUpdate.FullState(
                makeLoggedInState(rev = 2u).also { state ->
                    state.router = Router(Screen.ChatList, emptyList())
                },
            ),
        )
        waitFor("stale route reconciled with local push") {
            appManager.state.value.rev == 2uL
        }

        assertEquals(listOf(Screen.Settings), appManager.state.value.router.screenStack)
    }

    @Test
    fun persist_account_bundle_side_effect_applies_even_when_stale() {
        rustFactory.initialStates += makeLargeFixtureState(rev = 5u)
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
    fun export_owner_secret_reads_persisted_account_bundle() {
        val appManager = createManager()
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )

        val ownerNsec = runBlocking { appManager.exportOwnerNsec() }

        assertEquals("nsec1owner", ownerNsec)
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

        assertNull(ownerNsec)
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
    fun logout_does_not_delete_local_data_when_secure_secret_clear_fails() {
        rustFactory.initialStates += makeLoggedInState(rev = 5u)
        val appManager = createManager()
        val firstRust = rustFactory.instances.single()
        secureSecretStore.clearSucceeds = false
        persistStoredSecret(
            StoredAccountBundle(
                ownerNsec = "nsec1owner",
                ownerPubkeyHex = "owner-hex",
                deviceNsec = "nsec1device",
            ).toJson(),
        )
        val staleFile = appContext.filesDir.resolve("contract-logout-failed-${UUID.randomUUID()}.txt")
        staleFile.writeText("stale")

        appManager.logout()

        waitFor("secure clear failure") {
            secureSecretStore.clearCount == 1 &&
                appManager.state.value.toast == "Could not clear secret key."
        }

        assertFalse(firstRust.dispatchedActions.contains(AppAction.Logout))
        assertEquals(1, rustFactory.instances.size)
        assertEquals(0, firstRust.shutdownCount)
        assertTrue(staleFile.exists())
        assertEquals("nsec1device", loadPersistedBundle()?.deviceNsec)
        assertEquals(makeLoggedInState(rev = 5u).account, appManager.state.value.account)
        secureSecretStore.clearSucceeds = true
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
        currentChat: CurrentChatSnapshot? = null,
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
                    uploadProgress = null,
                ),
            chatList = emptyList(),
            currentChat = currentChat,
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
                    nearbyEnabled = true,
                    nearbyBluetoothEnabled = false,
                    nearbyLanEnabled = false,
                    nearbyShowInChatList = true,
                    nearbyMailbagEnabled = true,
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
                    pinnedChatIds = emptyList(),
                    blockedOwnerPubkeys = emptyList(),
                    acceptedOwnerPubkeys = emptyList(),
                    debugLoggingEnabled = false,
                    acceptUnknownDirectMessages = true,
                    mobilePushServerUrl = "",
                ),
            toast = toast,
        )

    private fun makeLargeFixtureState(
        rev: ULong = 1u,
        router: Router? = null,
        toast: String? = null,
        account: AccountSnapshot? = null,
        currentChat: CurrentChatSnapshot? = null,
    ): AppState =
        buildLargeTestAppState(
            directChatCount = 80u,
            groupChatCount = 20u,
            messagesInCurrentChat = LARGE_FIXTURE_MESSAGE_COUNT,
        ).also { state ->
            state.rev = rev
            state.preferences.nearbyBluetoothEnabled = false
            state.preferences.nearbyLanEnabled = false
            router?.let { state.router = it }
            account?.let { state.account = it }
            currentChat?.let { state.currentChat = it }
            state.toast = toast
        }

    private fun makeCurrentChat(
        chatId: String,
        kind: ChatKind,
        groupId: String? = null,
        messages: List<ChatMessageSnapshot> = emptyList(),
    ): CurrentChatSnapshot =
        CurrentChatSnapshot(
            chatId = chatId,
            kind = kind,
            displayName = "Chat",
            nickname = null,
            profileName = null,
            subtitle = null,
            pictureUrl = null,
            about = null,
            groupId = groupId,
            memberCount = 0u,
            messageTtlSeconds = null,
            isMuted = false,
            participants = emptyList(),
            messages = messages,
            typingIndicators = emptyList(),
            draft = "",
            isRequest = false, protocolReadiness = readyProtocolReadiness(),
        )

    private fun makeMessage(
        chatId: String,
        id: String,
        body: String = "message $id",
    ): ChatMessageSnapshot =
        ChatMessageSnapshot(
            id = id,
            chatId = chatId,
            kind = ChatMessageKind.USER,
            author = "owner-hex",
            authorOwnerPubkeyHex = "owner-hex",
            authorPictureUrl = null,
            body = body,
            attachments = emptyList(),
            reactions = emptyList(),
            reactors = emptyList(),
            isOutgoing = true,
            createdAtSecs = id.toULongOrNull() ?: 0u,
            expiresAtSecs = null,
            delivery = DeliveryState.SENT,
            recipientDeliveries = emptyList(),
            deliveryTrace =
                MessageDeliveryTraceSnapshot(
                    outerEventIds = emptyList(),
                    pendingRelayEventIds = emptyList(),
                    queuedProtocolTargets = emptyList(),
                    transportChannels = emptyList(),
                    lastTransportError = null,
                ),
            sourceEventId = null,
        )

    private fun makeLoggedInState(rev: ULong): AppState =
        makeLargeFixtureState(
            rev = rev,
            router = Router(Screen.ChatList, emptyList()),
            account =
                AccountSnapshot(
                    publicKeyHex = "owner-hex",
                    npub = "npub1owner",
                    displayName = "Owner",
                    pictureUrl = null,
                    about = null,
                    devicePublicKeyHex = "device-hex",
                    deviceNpub = "npub1device",
                    hasOwnerSigningAuthority = true,
                    authorizationState = DeviceAuthorizationState.AUTHORIZED, protocolReadiness = readyProtocolReadiness(),
                ),
        )

    private companion object {
        val LARGE_FIXTURE_MESSAGE_COUNT = 1_200u
        val SECRET_CIPHERTEXT = stringPreferencesKey("secret_ciphertext")
        val SECRET_IV = stringPreferencesKey("secret_iv")
    }
}

private class RecordingSecureSecretStore : SecureSecretStore {
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

    override fun search(query: String, scopeChatId: String?, limit: UInt): SearchResultSnapshot {
        val messageCount = if (limit > 120u) limit else 120u
        return buildLargeTestSearchResult(
            query = query,
            contactCount = 25u,
            groupCount = 9u,
            messageCount = messageCount,
        ).also { result ->
            result.scopeChatId = scopeChatId
            if (scopeChatId != null) {
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
        return CurrentChatSnapshot(
            chatId = trimmed,
            kind = thread?.kind ?: if (groupId == null) ChatKind.DIRECT else ChatKind.GROUP,
            displayName = thread?.displayName ?: trimmed,
            nickname = thread?.nickname,
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
            isRequest = thread?.isRequest ?: false, protocolReadiness = thread?.protocolReadiness ?: readyProtocolReadiness(),
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

    override fun ingestNearbyEventJson(eventJson: String): Boolean = true

    override fun ingestNearbyEventJsonWithTransport(eventJson: String, transport: String): Boolean = true

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
                    uploadProgress = null,
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
                    nearbyEnabled = true,
                    nearbyBluetoothEnabled = false,
                    nearbyLanEnabled = false,
                    nearbyShowInChatList = true,
                    nearbyMailbagEnabled = true,
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
                    pinnedChatIds = emptyList(),
                    blockedOwnerPubkeys = emptyList(),
                    acceptedOwnerPubkeys = emptyList(),
                    debugLoggingEnabled = false,
                    acceptUnknownDirectMessages = true,
                    mobilePushServerUrl = "",
                ),
            toast = null,
        )
}
