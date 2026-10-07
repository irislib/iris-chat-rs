package to.iris.chat

import android.os.Bundle
import to.iris.chat.rust.AccountSnapshot

internal class BleSavedIdentity(arguments: Bundle, packageName: String, createIfMissing: Boolean) {
    private val owner = arguments.getString("preserve_owner")
    private val devices = arguments.getString("preserve_devices")?.split(",")

    init {
        if (owner != null || devices != null) {
            require(packageName == "to.iris.chat.blegate" && !createIfMissing)
            require(owner?.matches(Regex("[a-f0-9]{64}")) == true)
            require(!devices.isNullOrEmpty() && devices.all { it.matches(Regex("[a-f0-9]{64}")) })
        }
    }

    fun checkAccount(account: AccountSnapshot) {
        if (owner != null) {
            check(account.publicKeyHex == owner && account.devicePublicKeyHex in devices.orEmpty()) {
                "Saved Bluetooth test account identity changed"
            }
        }
    }

    fun checkNeedsLogin() {
        check(owner == null) { "Saved Bluetooth test account could not be restored" }
    }
}
