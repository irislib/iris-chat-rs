package to.iris.chat.account

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidKeystoreSecretStoreTest {
    @Test
    fun encrypt_and_decrypt_roundtrip() {
        val store = AndroidKeystoreSecretStore()
        val input = ByteArray(32) { index -> index.toByte() }

        val encrypted = store.encrypt(input)
        val decrypted = store.decrypt(encrypted)

        assertArrayEquals(input, decrypted)
    }

    @Test
    fun clear_removes_key_material() {
        val store = AndroidKeystoreSecretStore()
        val input = ByteArray(32) { index -> index.toByte() }
        val encrypted = store.encrypt(input)

        assertTrue(store.clear())

        val decryptAfterClear = runCatching { store.decrypt(encrypted) }
        assertTrue(decryptAfterClear.isFailure)
    }
}
