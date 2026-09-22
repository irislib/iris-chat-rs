package to.iris.chat.push

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.chat.account.StoredAccountBundle

@RunWith(AndroidJUnit4::class)
class MobilePushCredentialsTest {
    @Test
    fun linkedDevicePushCredentialsSurvivePersistenceWithoutAccountSecret() {
        val linked = StoredAccountBundle(null, "account", "device-secret")
        val restored = requireNotNull(StoredAccountBundle.fromJson(linked.toJson()))
        assertNull(restored.ownerNsec)
        assertEquals("device-secret", restored.mobilePushAuthNsec)
        assertEquals("account-secret", linked.copy(ownerNsec = "account-secret").mobilePushAuthNsec)
    }
}
