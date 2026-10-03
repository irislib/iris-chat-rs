package to.iris.chat

internal fun harnessAttachmentSha256(bytes: ByteArray): String =
    java.security.MessageDigest
        .getInstance("SHA-256")
        .digest(bytes)
        .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
