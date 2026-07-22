package to.iris.chat.account

import org.json.JSONObject

data class AccountState(
    val publicKeyHex: String,
    val npub: String,
)

data class StoredAccountBundle(
    val ownerNsec: String?,
    val ownerPubkeyHex: String,
    val deviceNsec: String,
) {
    fun toJson(): String =
        JSONObject()
            .put("owner_nsec", ownerNsec)
            .put("owner_pubkey_hex", ownerPubkeyHex)
            .put("device_nsec", deviceNsec)
            .toString()

    companion object {
        fun fromJson(value: String): StoredAccountBundle? =
            runCatching {
                val json = JSONObject(value)
                StoredAccountBundle(
                    ownerNsec =
                        json.optString("owner_nsec")
                            .takeIf { it.isNotBlank() && it != "null" },
                    ownerPubkeyHex = json.getString("owner_pubkey_hex"),
                    deviceNsec = json.getString("device_nsec"),
                )
            }.getOrNull()
    }
}

data class StoredPendingDeviceLink(
    val deviceNsec: String,
    val approvalBootstrapJson: String,
) {
    fun toJson(): String =
        JSONObject()
            .put("device_nsec", deviceNsec)
            .put("approval_bootstrap_json", approvalBootstrapJson)
            .toString()

    companion object {
        fun fromJson(value: String): StoredPendingDeviceLink? =
            runCatching {
                val json = JSONObject(value)
                StoredPendingDeviceLink(
                    deviceNsec = json.getString("device_nsec"),
                    approvalBootstrapJson = json.getString("approval_bootstrap_json"),
                )
            }.getOrNull()
    }
}

data class EncryptedSecret(
    val cipherText: ByteArray,
    val iv: ByteArray,
)

sealed interface AccountBootstrapState {
    data object Loading : AccountBootstrapState
    data object NeedsLogin : AccountBootstrapState
    data class LoggedIn(val account: AccountState) : AccountBootstrapState
}
