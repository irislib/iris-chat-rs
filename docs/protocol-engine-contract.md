# Protocol Engine Contract

This document defines the protocol-engine behavior Iris intends to support.
It is a contract for refactoring and test coverage, not an implementation
guide for UI features.

The current protocol stack has three layers:

- `nostr-double-ratchet::SessionManager`: protocol state machine for invites,
  sessions, owner/device rosters, and encrypted deliveries.
- `ProtocolEngine`: Iris adapter around `SessionManager`. It translates AppKeys,
  Nostr events, pending retry state, and app send requests into protocol state
  changes and app-level effects.
- `AppCore`: executes effects by publishing Nostr events, fetching relay state,
  applying decrypted messages, updating chat state, and persisting app state.

The goal is to keep those responsibilities explicit and small.

## Identity Model

Iris has to handle three related identities:

| Name | Meaning | Example |
| --- | --- | --- |
| Owner key | User/account/app identity used as the chat identity. | Justin's main account key. |
| Device key | Concrete device/session identity used by NDR invites and sessions. | Justin's phone or desktop device key. |
| Session sender key | Key used inside an encrypted message envelope. | Message sender ratchet key. |

The main risky mapping is:

```text
device key -> owner key
```

When AppKeys are known, this mapping is verified by a roster. When they are not
known, the system may temporarily use a provisional fallback:

```text
owner key = device key
```

That fallback exists for legacy and single-key clients. It must not override a
verified AppKeys roster.

## Current Engine Lifecycle

1. A user logs in or restores a session.
2. `AppCore` clears previous runtime state and creates `ProtocolEngine`.
3. `ProtocolEngine::load_or_seed` loads persisted protocol state or seeds a new
   `SessionManager` and group manager.
4. The engine ensures a local invite and a local roster for the current device.
5. `AppCore` replays already known AppKeys into the engine.
6. Relay events feed the engine:
   - AppKeys event -> `ingest_app_keys_snapshot`
   - NDR invite event -> `observe_invite_event`
   - invite response event -> `observe_invite_response_event`
   - direct message event -> `process_direct_message_event`
   - group message event -> group processing paths
7. App actions ask the engine to prepare sends.
8. The engine returns `ProtocolEffect` values.
9. `AppCore` executes those effects and persists state.
10. Missing protocol state is queued and retried when AppKeys, invites, or
    subscription liveness events arrive.

## Supported Cases

Status values:

- `Supported`: expected behavior that should remain supported.
- `Supported, risky`: expected behavior, but code should become clearer and
  tests should be explicit.
- `Planned`: behavior we want, but coverage or implementation may be incomplete.
- `Unsupported`: behavior we should reject or ignore deliberately.

| ID | Case | Status | Expected Behavior | Required Coverage |
| --- | --- | --- | --- | --- |
| I01 | Single-key legacy peer, owner equals device. | Supported | Ownerless invite from `D` stores under owner `D`; sends to `D` can use that invite/session. | Unit test for ownerless invite with no AppKeys. |
| I02 | Multi-device peer with explicit owner hint in invite. | Supported | Invite from device `D` with owner `O` stores under `O`; mismatch between expected owner and invite owner is rejected. | Unit test for explicit owner and mismatch. |
| I03 | Ownerless invite from known peer device. | Supported, risky | If AppKeys say device `D` belongs to owner `O`, ownerless invite from `D` stores under `O`, not under `D`. | Regression test for AppKeys-before-ownerless-invite. |
| I04 | Ownerless invite from unknown device. | Supported | Store under provisional owner `D`; later verified AppKeys may need migration or reconciliation. | Unit test for unknown ownerless invite plus later AppKeys. |
| I05 | AppKeys arrive before invite. | Supported | Roster is installed first; later invite is resolved against known owner/device mapping. | Matrix test. |
| I06 | Invite arrives before AppKeys. | Supported, risky | Pending sends may queue for roster or device invite; later AppKeys should wake and retry. | Matrix test for both arrival orders. |
| I07 | Send to peer with roster but missing device invite/session. | Supported | Queue outbound work for missing device invite; fetch invite by device author. | Unit test for missing invite gap. |
| I08 | Send to peer with missing roster. | Supported | Queue outbound work for missing roster; fetch AppKeys by owner author. | Unit test for missing roster gap. |
| I09 | Peer has multiple authorized devices. | Supported | Prepare delivery for every authorized, non-stale device with a usable session/invite; queue gaps for missing devices. | Multi-device fanout test. |
| I10 | Peer device revoked or stale. | Supported | Do not prepare new delivery for revoked/stale device. Existing pending work should not resurrect it. | Revocation regression test. |
| I11 | Local owner has linked local sibling devices. | Supported, risky | Send local sibling copies separately from remote peer deliveries; do not route sibling copies as remote chat messages. | Local sibling send tests. |
| I12 | Linked local sibling is missing roster/session state. | Supported, risky | Queue or probe local sibling state without blocking remote delivery. | Local sibling gap tests. |
| I13 | First-contact direct send. | Supported, risky | Publish invite response/bootstrap before payload; stage payload publishing briefly. | Staged first-contact test with relay assertions. |
| I14 | Duplicate invite or invite response. | Supported | Ignore duplicates or use newest valid invite without corrupting sessions. | Idempotency tests. |
| I15 | Used or exhausted invite. | Supported | Do not crash; treat as unavailable and queue/fetch if needed. | Unit test around `InviteAlreadyUsed` and `InviteExhausted`. |
| I16 | Malicious owner claim not backed by AppKeys. | Supported | Treat as pending/unverified; do not accept claimed owner as verified until roster proves it. | Adversarial test. |
| I17 | Incoming message from unknown sender/session. | Supported | Store pending inbound, fetch relevant protocol state, retry when state arrives. | Pending inbound test. |
| I18 | Incoming group sender-key message arrives after group outer. | Supported | Queue/defer group outer until pairwise sender key material arrives. | Existing sender-key tests, plus matrix row. |
| I19 | Group membership changes remove a sender. | Supported | Removed member should not be able to create valid new group mutations or sender keys. | Group hardening tests. |
| I20 | Public relay replays already-seen protocol event. | Supported | Replay only when needed to satisfy queued protocol work; otherwise avoid duplicate processing. | Seen-event replay test. |

## Failure Scenarios

| ID | Failure | Expected Behavior | Required Coverage |
| --- | --- | --- | --- |
| F01 | Relay publish rejects a protocol event. | Keep publish pending or mark message failed according to publish semantics; do not advance protocol state twice. | Test relay `reject_next`. |
| F02 | Relay accepts publish but subscriber misses it. | Receiver should recover through replay/backfill if event is still available. | Test relay delivery drop plus backfill. |
| F03 | App restarts with pending outbound work. | Pending work reloads and retries without duplicating message state. | Restart test. |
| F04 | App restarts with pending inbound unresolved event. | Pending inbound reloads and retries when missing protocol state arrives. | Restart test. |
| F05 | AppKeys event is stale. | Ignore stale roster, keep newer known roster, do not resurrect devices. | Stale AppKeys test. |
| F06 | AppKeys event is same timestamp with changed devices. | Merge according to roster policy; behavior must be explicit. | Same-timestamp merge test. |
| F07 | AppKeys event omits current local device during restore. | Preserve or republish required local device according to restore mode. | Restore-specific test. |
| F08 | Invite owner hint conflicts with resolved owner. | Reject or ignore the invite; never silently store under wrong owner. | Mismatch test. |
| F09 | Ownerless invite conflicts with verified roster. | Verified roster wins; store under roster owner. | Ownerless known-roster regression. |
| F10 | Ownerless invite has no matching roster. | Legacy fallback `owner=device` is allowed. | Legacy compatibility test. |
| F11 | Device appears in multiple peer rosters. | Prefer verified non-local owner only if unambiguous; otherwise treat as conflict/pending. | Conflict test, policy needed. |
| F12 | Missing roster and missing invite both block a send. | Queue exact missing requirements and emit fetch filters for both. | Pending requirement test. |
| F13 | Sender-key repair cannot be authorized. | Do not leak repair material; queue or reject according to group policy. | Sender-key repair auth test. |
| F14 | Protocol effect publish succeeds after delay. | Completion updates the intended message once, using inner event id metadata where needed. | Runtime publish completion test. |
| F15 | Protocol subscription is stale or disconnected. | Liveness check should resubscribe and retry queued work without tight loops. | Liveness/reconnect test. |

## Desired Refactoring Shape

The current code spreads ownership, roster, invite, send planning, retry, and
effect emission decisions across several files. We should reduce that by making
the state machines explicit.

Target modules:

| Module | Responsibility |
| --- | --- |
| `OwnerResolver` | Resolve device and sender identities into verified, provisional, pending, or unknown owner states. |
| `RosterStore` | Own current owner/device roster knowledge and stale/revoked policy. |
| `InviteStore` | Own public invites by owner and device. |
| `SendPlanner` | Given owner and payload, produce deliveries or missing requirements. |
| `PendingQueue` | Store missing requirements and retry scheduling. |
| `EffectEmitter` | Convert prepared sends and retry results into `ProtocolEffect` values. |

The first extraction should be `OwnerResolver`, because it addresses the class
of bugs around ownerless invites, device keys, owner hints, and AppKeys rosters.

## Desired Types

Prefer explicit result types over returning a bare owner key.

```rust
enum OwnerResolution {
    Verified {
        owner: NdrOwnerPubkey,
    },
    ProvisionalDeviceOwner {
        owner: NdrOwnerPubkey,
    },
    PendingOwnerClaim {
        storage_owner: NdrOwnerPubkey,
        claimed_owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
    },
    UnknownDevice {
        device: NdrDevicePubkey,
    },
    Conflict {
        device: NdrDevicePubkey,
        owners: Vec<NdrOwnerPubkey>,
    },
}

enum MissingProtocolState {
    Roster {
        owner: NdrOwnerPubkey,
    },
    DeviceInvite {
        owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
    },
    OwnerClaim {
        owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
    },
    SenderKeyRepair {
        group_id: String,
        sender_event_pubkey_hex: String,
        key_id: u32,
        message_number: u32,
    },
}
```

Call sites should be forced to handle `Verified` and `ProvisionalDeviceOwner`
differently where that matters.

## Test Strategy

Keep scenario tests, but add smaller contract tests.

### Unit Contract Tests

These should not require sockets or UI state. They should feed protocol inputs
directly into the engine or extracted modules.

Minimum initial set:

- ownerless invite with no AppKeys uses `owner=device`
- ownerless invite with known AppKeys uses roster owner
- explicit owner invite stores under explicit owner
- owner mismatch is rejected
- AppKeys before invite
- invite before AppKeys
- send with missing roster queues `MissingProtocolState::Roster`
- send with missing device invite queues `MissingProtocolState::DeviceInvite`
- stale roster does not resurrect revoked device
- pending outbound retries after AppKeys arrives
- pending outbound retries after device invite arrives
- pending inbound retries after owner claim becomes verified

### Integration Tests With Local Relay

Use the local test relay for app-level behavior:

- relay rejects bootstrap publish
- relay rejects payload publish
- relay drops AppKeys event
- relay drops invite event
- relay drops first-contact payload delivery
- receiver reconnects and recovers from stored events
- sender restarts with pending protocol publish
- receiver restarts with pending inbound direct event

### Public Relay Smoke Tests

Keep these few and slow:

- legacy/plain invite interoperability
- multi-device AppKeys interoperability
- restart plus backfill on public relays

Do not use public relays for deterministic state-machine coverage.

## Refactoring Order

1. Add matrix tests for owner/device/invite resolution using current code.
2. Extract `OwnerResolver` with no behavior change.
3. Replace stringly queued targets with `MissingProtocolState`.
4. Extract pending retry/fetch filter generation.
5. Simplify send paths so direct, local sibling, and group fanout share the same
   missing-state model.
6. Move effect emission into a small conversion layer.
7. Remove dead or redundant retry branches once matrix coverage is green.

## Non-Goals

- Do not rewrite `SessionManager` and `ProtocolEngine` together.
- Do not change wire formats as part of cleanup.
- Do not remove legacy ownerless invite support.
- Do not make production relay behavior part of deterministic CI.
- Do not depend on UI E2E for protocol-state correctness.

## Acceptance Criteria For Cleanup

The cleanup is not done until:

- every supported case above has at least one named test
- every failure scenario above is either tested or explicitly marked unsupported
- owner/device resolution is represented by explicit types
- pending retry state records missing protocol requirements directly
- AppKeys, invite, send, receive, and retry flows each have a short file-level
  explanation
- protocol-engine LOC is reduced or split into smaller modules with clear
  ownership boundaries
