package to.iris.chat.account

interface SecureSecretStore {
    fun encrypt(secret: ByteArray): EncryptedSecret

    fun decrypt(encryptedSecret: EncryptedSecret): ByteArray

    fun clear(): Boolean
}
