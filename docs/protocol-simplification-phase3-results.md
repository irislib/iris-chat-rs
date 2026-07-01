# Protocol Simplification Phase 3 Results

## Summary

Phase 3 verifies that readiness can converge from state-derived relay subscriptions after the Phase 1 backfill/retry removal and Phase 2 explicit readiness gate.

No public readiness types or native bindings are changed in this phase.

## Readiness Convergence Map

| Readiness state | Subscription interest expected to resolve it | Phase 3 status |
| --- | --- | --- |
| `AccountMissing` | None. User must create or restore an account. | Not subscription-resolvable. |
| `DeviceAwaitingApproval` | Owner AppKeys and invite-response subscriptions for linked-device approval state. | Existing behavior retained; not expanded in Phase 3. |
| `DeviceRevoked` | Owner AppKeys can confirm revocation, but send remains blocked. | Existing behavior retained. |
| `ProtocolEngineUnavailable` | None. Runtime/session startup must restore the engine. | Not subscription-resolvable. |
| `BlockedPeer` | None. User preference blocks send and removes peer from protocol interest. | Not subscription-resolvable. |
| `PeerAppKeysMissing` | AppKeys subscription for the tracked peer owner. | Covered by `direct_chat_readiness_converges_from_appkeys_and_invite_events`. |
| `PeerSessionMissing` | Invite subscription for known peer device authors, plus local message-recipient bootstrap while no message author session exists. | Covered by `direct_chat_readiness_converges_from_appkeys_and_invite_events`. |
| `GroupMetadataMissing` | Group metadata delivered through existing group pairwise protocol control. | Covered by `group_readiness_converges_when_metadata_arrives`. |
| `GroupNotJoined` | None for send-readiness. Future group metadata can change membership, but the current state must block. | Covered by `group_readiness_converges_when_metadata_arrives`. |

## Production Mechanisms Exercised

- Direct chat convergence is driven by relay-delivered AppKeys and invite events, handled through `AppCore::handle_relay_event`.
- CLI first contact now proves the user-visible transition: send is blocked before relay state exists, `sync` receives the state, and send succeeds after readiness.
- Group metadata convergence uses the existing sender-key pairwise control path to produce `MetadataUpdated`, then verifies AppCore readiness projection.
- Signed relay publish retry remains the only retry queue used after a ready send.

## Deferred To Phase 4

- Full group readiness for member AppKeys, sender-key authors, linked-device sender-key sync, and receive-side sender-key state.
- Cleanup of stale debug names and no-op retry/backfill compatibility fields.
- Any narrow explicit fetch/sync mechanism, if a future test proves subscriptions cannot provide required current state.

## Verification

Completed commands:

```sh
cd /Users/l/Projects/iris-core-architecture/iris-chat-rs/chat-protocol
cargo test
cd /Users/l/Projects/iris-core-architecture/iris-chat-rs/core
cargo test
```

Final status: passed.
