package to.iris.chat.account

import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class AccountStorageTest {
    @get:Rule val temporary = TemporaryFolder()

    @Test
    fun newProfileLeavesLegacyDatabaseAndCredentialsUntouchedAndSurvivesRestart() {
        val legacy = temporary.newFolder("files")
        val noBackup = temporary.newFolder("no-backup")
        val database = File(legacy, "iris.sqlite").apply { writeText("old account and chats") }
        val credentials = File(legacy, "datastore/account.preferences_pb").apply {
            parentFile!!.mkdirs()
            writeText("encrypted old key")
        }
        val storage = AccountStorage(legacy, noBackup)
        assertEquals(legacy, storage.currentDirectory())

        val fresh = storage.prepareNewProfile()

        assertTrue(fresh.listFiles()!!.isEmpty())
        assertNotEquals(legacy, fresh)
        assertEquals("old account and chats", database.readText())
        assertEquals("encrypted old key", credentials.readText())
        assertEquals(fresh, AccountStorage(legacy, noBackup).currentDirectory())
    }

    @Test
    fun repeatedSignupPreservesTheEntirePreviousStoreIncludingWalAndAttachments() {
        val legacy = temporary.newFolder("files")
        val noBackup = temporary.newFolder("no-backup")
        val storage = AccountStorage(legacy, noBackup)
        val active = storage.prepareNewProfile()
        File(active, "iris.sqlite").writeText("account")
        File(active, "iris.sqlite-wal").writeText("latest messages")
        File(active, "attachment").writeText("photo")

        assertEquals(active, storage.prepareNewProfile())

        assertTrue(active.listFiles()!!.isEmpty())
        val recovered = active.parentFile!!.listFiles()!!.single { it != active }
        assertEquals("account", File(recovered, "iris.sqlite").readText())
        assertEquals("latest messages", File(recovered, "iris.sqlite-wal").readText())
        assertEquals("photo", File(recovered, "attachment").readText())
    }

    @Test
    fun interruptedPreparationDoesNotReopenLegacyAccount() {
        val legacy = temporary.newFolder("files")
        val noBackup = temporary.newFolder("no-backup")
        File(noBackup, "profiles").mkdirs()
        assertEquals(File(noBackup, "profiles/active"), AccountStorage(legacy, noBackup).currentDirectory())
    }
}
