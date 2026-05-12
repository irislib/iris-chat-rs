package to.iris.chat.ui.screens

import to.iris.chat.rust.SearchResultSnapshot

internal val InitialMessageSearchLimit: UInt = 50u
private val MessageSearchLimitStep: UInt = 50u

internal fun nextMessageSearchLimit(current: UInt): UInt =
    if (UInt.MAX_VALUE - current < MessageSearchLimitStep) {
        UInt.MAX_VALUE
    } else {
        current + MessageSearchLimitStep
    }

internal fun SearchResultSnapshot.matchesSearchRequest(
    query: String,
    scopeChatId: String? = null,
): Boolean = this.query.trim() == query && this.scopeChatId == scopeChatId
