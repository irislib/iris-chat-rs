# Protocol Simplification Phase 4 Results

## Group Send Readiness

Phase 4 makes group send readiness explicit. A group can send only when:

- account readiness is `Ready`
- group metadata is present
- the local owner is a current member
- every current non-local member has known AppKeys
- every current non-local member has direct send capability

Missing member AppKeys now reports `GroupMemberAppKeysMissing`. Known member AppKeys without a usable direct session reports `GroupMemberSessionMissing`.

Local sender-key state is not a readiness prerequisite. The group send path remains responsible for creating or using sender-key state and publishing signed distributions when the group is otherwise ready.

## Subscription Convergence

Known group members already feed the state-derived protocol subscription plan through tracked protocol owners. Phase 4 tests cover this convergence:

- missing group member AppKeys keep the member owner in roster subscriptions
- known member devices enter invite-author subscriptions
- local message-recipient bootstrap remains active until member sessions exist
- sends remain blocked until the subscription-derived protocol state makes the group ready

No procedural protocol fetch, backfill, queued-target retry, or sender-key repair loop was added.

## Receive Diagnostics

Missing receive-side sender-key state is diagnostic only. A consumed but pending group outer event now writes an AppCore debug log entry and stops there.

The diagnostic path does not create relay publishes, repair requests, queued protocol work, group fanouts, or retry rows.

## Remaining Work

Phase 5 should decide whether receive readiness needs public UI state, whether group membership should account for blocked owners, and whether stale group membership or linked-device sender-key sync need additional explicit readiness signals.
