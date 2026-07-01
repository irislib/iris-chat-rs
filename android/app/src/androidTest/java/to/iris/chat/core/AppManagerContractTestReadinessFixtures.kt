package to.iris.chat.core

import to.iris.chat.rust.ProtocolReadinessReason
import to.iris.chat.rust.ProtocolReadinessSnapshot

internal fun readyProtocolReadiness(): ProtocolReadinessSnapshot =
    ProtocolReadinessSnapshot(
        canSend = true,
        reason = ProtocolReadinessReason.READY,
        message = "Ready",
    )
