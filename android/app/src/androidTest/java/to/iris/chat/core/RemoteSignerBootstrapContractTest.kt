package to.iris.chat.core

import android.os.SystemClock
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.account.AccountBootstrapState
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.RemoteSignerLoginSnapshot
import to.iris.chat.rust.RemoteSignerPhase
import to.iris.chat.rust.Router
import to.iris.chat.rust.Screen

@RunWith(AndroidJUnit4::class)
class RemoteSignerBootstrapContractTest {
    @Test
    fun interactive_signer_login_keeps_approval_controls_visible() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val storeName = "signer-bootstrap-${UUID.randomUUID()}.preferences_pb"
        val store = PreferenceDataStoreFactory.create(scope = scope) { context.preferencesDataStoreFile(storeName) }
        val factory = RecordingRustFactory()
        val manager = AppManager(
            context = context,
            applicationScope = scope,
            secureSecretStore = RecordingSecureSecretStore(),
            ioDispatcher = Dispatchers.IO,
            dataStoreName = storeName,
            dataStore = store,
            rustFactory = { _, _ -> factory.create() },
        )
        try {
            waitFor { manager.bootstrapState.value is AccountBootstrapState.NeedsLogin }
            val rust = factory.instances.single()
            val connecting = rust.currentState.copy(
                rev = 1u,
                router = Router(Screen.Welcome, listOf(Screen.RestoreAccount, Screen.RemoteSigner)),
                busy = rust.currentState.busy.copy(restoringSession = true),
                remoteSignerLogin = RemoteSignerLoginSnapshot("nostrconnect://test", RemoteSignerPhase.CONNECTING, null),
            )
            rust.emit(AppUpdate.FullState(connecting))
            waitFor { manager.state.value.rev == 1uL }
            assertTrue(manager.bootstrapState.value is AccountBootstrapState.NeedsLogin)

            // Handing off to a local signer still uses the same interactive screen.
            rust.emit(AppUpdate.FullState(connecting.copy(rev = 2u, remoteSignerLogin = null)))
            waitFor { manager.state.value.rev == 2uL }
            assertTrue(manager.bootstrapState.value is AccountBootstrapState.NeedsLogin)

            // Ordinary key restoration retains the existing loading screen.
            rust.emit(AppUpdate.FullState(connecting.copy(
                rev = 3u,
                router = Router(Screen.Welcome, listOf(Screen.RestoreAccount)),
                remoteSignerLogin = null,
            )))
            waitFor { manager.bootstrapState.value is AccountBootstrapState.Loading }
        } finally {
            manager.resetForUiTestsBlocking()
            scope.cancel()
            context.preferencesDataStoreFile(storeName).delete()
        }
    }

    private fun waitFor(predicate: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 10_000
        while (!predicate()) {
            check(SystemClock.elapsedRealtime() < deadline) { "Bootstrap did not settle" }
            SystemClock.sleep(25)
        }
    }
}
