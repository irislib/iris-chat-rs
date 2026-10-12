package to.iris.chat.account

import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import to.iris.chat.rust.AppAction
import to.iris.chat.rust.AppReconciler
import to.iris.chat.rust.AppUpdate
import to.iris.chat.rust.FfiApp

class AccountStorageIntegrationTest {
    @get:Rule val temporary = TemporaryFolder()

    @Test
    fun strandedDatabaseRejectsSignupButNewProfileCreatesAndRestoresWithoutChangingIt() {
        val legacy = temporary.newFolder("files")
        val noBackup = temporary.newFolder("no-backup")
        val storage = AccountStorage(legacy, noBackup)
        val oldOwner = withCore(legacy) { core ->
            core.dispatch(AppAction.CreateAccount("Previous account"))
            await { core.state().account != null }
            core.state().account!!.publicKeyHex
        }

        // Reinstall/restore retained the database but lost its Keystore key.
        // Reproduce the exact error reported from the signup screen.
        withCore(storage.currentDirectory()) { core ->
            assertNull(core.state().account)
            core.dispatch(AppAction.CreateAccount("New account"))
            await { core.state().toast != null }
            assertTrue(core.state().toast!!.contains("different account"))
            assertNull(core.state().account)
        }
        val originalDatabase = File(legacy, "core.sqlite3").readBytes()

        val updates = CopyOnWriteArrayList<AppUpdate>()
        val fresh = storage.prepareNewProfile()
        val newOwner = withCore(fresh) { core ->
            core.listenForUpdates(object : AppReconciler {
                override fun reconcile(update: AppUpdate) { updates += update }
            })
            core.dispatch(AppAction.CreateAccount("New account"))
            await { core.state().account != null && updates.any { it is AppUpdate.PersistAccountBundle } }
            assertNull(core.state().toast)
            core.state().account!!.publicKeyHex
        }
        assertNotEquals(oldOwner, newOwner)
        assertArrayEquals(originalDatabase, File(legacy, "core.sqlite3").readBytes())

        val saved = updates.filterIsInstance<AppUpdate.PersistAccountBundle>().last()
        withCore(AccountStorage(legacy, noBackup).currentDirectory()) { core ->
            core.dispatch(AppAction.RestoreAccountBundle(saved.ownerNsec, saved.ownerPubkeyHex, saved.deviceNsec))
            await { core.state().account != null }
            assertEquals(newOwner, core.state().account!!.publicKeyHex)
            assertNull(core.state().toast)
        }
    }

    private fun <T> withCore(directory: File, block: (FfiApp) -> T): T {
        val core = FfiApp(directory.absolutePath, "", "test")
        try {
            core.dispatch(AppAction.SetNostrRelays(emptyList()))
            core.dispatch(AppAction.SetNearbyLanEnabled(false))
            core.dispatch(AppAction.SetNearbyBluetoothEnabled(false))
            return block(core)
        } finally {
            core.shutdownAndWait()
            core.close()
        }
    }

    private fun await(condition: () -> Boolean) {
        val deadline = System.nanoTime() + 10_000_000_000L
        while (!condition()) {
            check(System.nanoTime() < deadline) { "Timed out waiting for account state" }
            Thread.sleep(10)
        }
    }
}
