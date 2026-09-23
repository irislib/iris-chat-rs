package to.iris.chat.core

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn

/** Keep each UI subscription current without invalidating it for unrelated changes. */
internal fun <State, Slice> StateFlow<State>.slice(
    scope: CoroutineScope,
    select: (State) -> Slice,
): StateFlow<Slice> = map(select).distinctUntilChanged().stateIn(
    scope = scope,
    started = SharingStarted.Eagerly,
    initialValue = select(value),
)
