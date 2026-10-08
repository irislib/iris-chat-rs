package to.iris.chat.core

import to.iris.chat.rust.ChatMessageSnapshot

internal fun compareChatMessages(lhs: ChatMessageSnapshot, rhs: ChatMessageSnapshot): Int {
    lhs.createdAtSecs.compareTo(rhs.createdAtSecs).takeIf { it != 0 }?.let { return it }
    val lhsNumeric = lhs.id.toULongOrNull()
    val rhsNumeric = rhs.id.toULongOrNull()
    if (lhsNumeric != null && rhsNumeric != null && lhsNumeric != rhsNumeric) {
        return lhsNumeric.compareTo(rhsNumeric)
    }
    if (lhsNumeric != null && rhsNumeric == null) {
        return -1
    }
    if (lhsNumeric == null && rhsNumeric != null) {
        return 1
    }
    return lhs.id.compareTo(rhs.id)
}
