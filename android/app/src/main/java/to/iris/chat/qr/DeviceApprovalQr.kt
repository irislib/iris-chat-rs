package to.iris.chat.qr

import to.iris.chat.rust.isDeviceApprovalBootstrap

object DeviceApprovalQr {
    fun isValid(raw: String): Boolean = isDeviceApprovalBootstrap(raw.trim())
}
