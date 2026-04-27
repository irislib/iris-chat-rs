package to.iris.chat.qr

import to.iris.chat.rust.DeviceApprovalQrPayload
import to.iris.chat.rust.decodeDeviceApprovalQr
import to.iris.chat.rust.encodeDeviceApprovalQr

object DeviceApprovalQr {
    fun encode(
        ownerInput: String,
        deviceInput: String,
    ): String = encodeDeviceApprovalQr(ownerInput.trim(), deviceInput.trim())

    fun decode(raw: String): DeviceApprovalQrPayload? = decodeDeviceApprovalQr(raw)
}
