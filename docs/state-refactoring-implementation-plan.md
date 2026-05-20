# State Refactoring Implementation Plan

Branch: `state-refactoring`

Base analyzed: `origin/main` at `ce5cb7f`

This plan turns the local-invite ownership design into an incremental implementation path that leads toward the broader protocol-engine contract.

## Scope

The first implementation milestone is deliberately narrow:

- keep `PendingLinkedDeviceState` as the owner of the temporary pre-login pairing invite
- make `ProtocolEngine` / `SessionManager` the only owner of the stable logged-in local invite
- remove `LoggedInState.local_invite`
- update tests so app-facing consumers prove they read the protocol-owned invite

The later milestones use the same pattern to tackle the larger state-machine cleanup:

- explicit owner/device resolution
- typed missing protocol state
- smaller send planning and retry boundaries

## Static Code Analysis

### Current State Holders

`AppCore` currently keeps the relevant state in four places:

- `logged_in: Option<LoggedInState>`
- `protocol_engine: Option<ProtocolEngine>`
- `pending_linked_device: Option<PendingLinkedDeviceState>`
- `private_chat_invites: BTreeMap<String, Invite>`

`LoggedInState` currently mixes concerns:

- login/account identity: `owner_pubkey`, `owner_keys`, `device_keys`
- relay runtime: `client`, `relay_urls`
- protocol identity: `local_invite`
- authorization: `authorization_state`

The problematic field is `local_invite`. Stable local invite state already exists inside `SessionManager`, reached through `ProtocolEngine`. `LoggedInState.local_invite` is therefore a second source of truth.

`PendingLinkedDeviceState` is different. It exists before login and before `ProtocolEngine` exists:

- it generates the future device key
- it creates an ownerless one-use pairing invite
- it runs a temporary Nostr client to wait for an invite response

That temporary invite is legitimate in pending state. It should not be confused with the stable local invite after login completes.

### Duplication Point

`AppCore::start_session` currently:

1. creates/loads a stable owner-bound local invite
2. passes a clone to `ProtocolEngine::load_or_seed`
3. stores the original in `LoggedInState.local_invite`

`ProtocolEngine::load_or_seed` then loads persisted protocol state and only installs the passed invite if `SessionManager` has no local invite. This means restore can already make `SessionManager` authoritative while the login copy remains whatever `AppCore` loaded separately.

That is the exact drift risk.

### Current Stable Invite Consumers

These app-layer paths currently read `LoggedInState.local_invite`:

- public invite snapshot fallback
- link-device snapshot while a linked device is awaiting approval
- local identity artifact publishing
- protocol invite-response filters
- mobile-push invite-response filters
- test helpers

These protocol paths already read `SessionManager.local_invite`:

- invite-response processing
- protocol-level tests through `local_invite_for_test`

The refactor should move every stable invite consumer to the protocol path.

### Current Test Coverage

Existing tests cover a lot of behavior around invites:

- pending linked-device flow creates an ownerless link invite
- pending linked-device flow completes when the owner accepts
- recent protocol filters include the local invite response pubkey
- private invites do not reuse the stable local invite secret
- mobile push includes local and private invite-response pubkeys
- protocol tests use `SessionManager` local invite through `local_invite_for_test`

The main gap is that most app-facing tests still derive expectations from `LoggedInState.local_invite`. They would pass even if the login copy and protocol copy drifted.

## Test-First Additions

I added ignored executable specs before implementing the refactor:

- `app_invite_consumers_use_protocol_owned_local_invite`
- `local_identity_publish_uses_protocol_owned_local_invite`
- `completed_pairing_discards_pairing_invite_and_creates_stable_local_invite`

The first two intentionally build a core where `LoggedInState.local_invite` and `ProtocolEngine`'s local invite differ. They assert that public invite projection, mobile push, protocol filters, and identity publishing use the protocol-owned invite. They are ignored because the current code still reads the login copy.

The pairing test asserts the conceptual lifecycle:

- pending pairing invite is ownerless and `purpose = "link"`
- after completion, pending state is gone
- protocol engine has a stable owner-bound invite
- the stable invite does not reuse the pairing-only invite material

When implementation starts, unignore the relevant test, make it fail, implement the smallest change, and repeat.

## Reference Implementations

### Signal / libsignal

Reference inspected:

- `signalapp/libsignal` commit `7c8cb0c5fce1d01805199de992bf4323f4765f1f`
- [Swift in-memory protocol store](https://github.com/signalapp/libsignal/blob/7c8cb0c5fce1d01805199de992bf4323f4765f1f/swift/Sources/LibSignalClient/DataStoreInMemory.swift)
- [Rust bridge storage traits](https://github.com/signalapp/libsignal/blob/7c8cb0c5fce1d01805199de992bf4323f4765f1f/rust/bridge/shared/types/src/protocol/storage.rs)

Relevant pattern:

- identity keys, sessions, prekeys, signed prekeys, kyber prekeys, and sender keys are modeled as store interfaces
- a concrete in-memory store may implement several traits, but the maps remain separate by concept
- session operations consume `SessionStore` and `IdentityKeyStore` rather than keeping duplicate session copies in app-login state
- identity trust is an explicit store decision, not inferred from a UI account object

Lesson for Iris:

- `LoggedInState` can contain the keys required to construct stores/runtimes, but stable protocol material should be owned by the protocol store/state machine
- app-level consumers should ask the protocol adapter for derived data instead of copying protocol records into login state

### Matrix Rust SDK

Reference inspected:

- `matrix-org/matrix-rust-sdk` commit `4a26af89f28c22d21c62c9a064b3d175e96020fd`
- [OlmMachine state fields](https://github.com/matrix-org/matrix-rust-sdk/blob/4a26af89f28c22d21c62c9a064b3d175e96020fd/crates/matrix-sdk-crypto/src/machine/mod.rs)
- [SenderDataFinder](https://github.com/matrix-org/matrix-rust-sdk/blob/4a26af89f28c22d21c62c9a064b3d175e96020fd/crates/matrix-sdk-crypto/src/olm/group_sessions/sender_data_finder.rs)

Relevant pattern:

- `OlmMachine` owns the crypto/session state machines and store
- store wrappers are explicit and shared through `Arc`
- sender/device resolution is documented as an algorithm with explicit outcomes like unknown device and failed owner check
- tests cover many resolution branches through structured setup options

Lesson for Iris:

- `ProtocolEngine` should be the app-facing owner of protocol/session state
- owner/device resolution should become a small explicit algorithm, not a chain of fallbacks returning a bare owner key
- the future `OwnerResolver` should return typed states like verified, provisional, pending, and conflict

### Pika Backup

Reference inspected:

- GNOME Pika Backup commit `63a1ac57133d29b679905264c0b969e303acbc9b`
- [application startup/shutdown](https://gitlab.gnome.org/World/pika-backup/-/blob/63a1ac57133d29b679905264c0b969e303acbc9b/pika-backup/src/app.rs)
- [global config/runtime state](https://gitlab.gnome.org/World/pika-backup/-/blob/63a1ac57133d29b679905264c0b969e303acbc9b/pika-backup/src/globals.rs)
- [operation runtime object](https://gitlab.gnome.org/World/pika-backup/-/blob/63a1ac57133d29b679905264c0b969e303acbc9b/pika-backup/src/operation.rs)
- [setup dialog transient state](https://gitlab.gnome.org/World/pika-backup/-/blob/63a1ac57133d29b679905264c0b969e303acbc9b/pika-backup/src/widget/dialog/setup.rs)

Relevant pattern:

- durable config/state and transient operation/UI state are intentionally separate
- long-running operations are represented by typed operation objects, not folded into app login/config state
- UI setup flows keep local `RefCell`/`Cell` transient fields rather than storing partial setup state in durable config

Lesson for Iris:

- `PendingLinkedDeviceState` is a transient operation state and should stay separate from durable logged-in protocol identity
- renaming pending fields to `pairing_invite`, `pairing_client`, and `pairing_url` would match this separation better

## Idiomatic Rust Direction

Use Rust's type system to enforce ownership:

- remove duplicated fields instead of relying on comments
- expose small accessors for derived state
- return typed result enums for owner/device resolution
- avoid stringly typed queued targets
- prefer helper structs with narrow responsibilities over large state bags
- keep test helpers aligned with production ownership so tests cannot accidentally synchronize duplicate state for us

For local invite access, start with owned returns:

```rust
impl ProtocolEngine {
    pub(super) fn local_invite(&self) -> Option<Invite>;
    pub(super) fn local_invite_response_pubkey(&self) -> Option<PublicKey>;
}
```

This is pragmatic because the current `SessionManager` API exposes snapshots. If the library later offers borrowed access, we can reduce cloning.

## Implementation Milestones

### Milestone 0: Test Harness

Status: started.

- add ignored drift tests for app-facing invite consumers
- add ignored pairing lifecycle test
- keep existing tests green by default
- run ignored tests manually to confirm they fail/pass for the intended reason

Exit criteria:

- ignored tests compile
- at least one drift test fails against current code for the expected reason

### Milestone 1: Protocol Invite Accessors

Add:

```rust
ProtocolEngine::local_invite()
ProtocolEngine::local_invite_response_pubkey()
```

Then add optional `AppCore` helpers:

```rust
AppCore::local_protocol_invite()
AppCore::local_protocol_invite_response_pubkey()
```

Exit criteria:

- no call site changes yet
- no behavior change
- tests compile

### Milestone 2: Move App Consumers To Protocol Accessors

Update stable invite consumers:

- `build_public_invite_snapshot`
- `build_link_device_snapshot`
- `publish_local_identity_artifacts`
- `protocol_invite_response_pubkeys`
- `build_mobile_push_sync_snapshot`

Important behavior:

- profile metadata and AppKeys publishing should not be skipped just because no local invite is available
- only the invite event should be gated on protocol invite availability

Exit criteria:

- unignore `app_invite_consumers_use_protocol_owned_local_invite`
- unignore `local_identity_publish_uses_protocol_owned_local_invite`
- both pass
- existing invite/filter/mobile-push tests still pass

### Milestone 3: Remove `LoggedInState.local_invite`

Delete the field.

Update every direct `LoggedInState` construction in tests to use helpers that install a protocol engine.

Exit criteria:

- `rg "logged_in.*local_invite|\\.local_invite" core/src/core` shows no stable login invite reads
- test helpers no longer pass a local invite through `LoggedInState`
- compiler prevents reintroduction

### Milestone 4: Move Stable Invite Creation Into Protocol Ownership

Move `load_or_create_local_invite` out of `account.rs`.

Preferred shape:

```rust
ProtocolEngine::load_or_seed_for_local_device(
    storage,
    owner_pubkey,
    device_keys,
    seed_session_manager,
    seed_group_manager,
)
```

This method should:

- derive device id from `device_keys.public_key()`
- load or create the owner-bound local invite
- install it into `SessionManager` only if missing
- if persisted `SessionManager` already has a local invite, use that invite as authoritative for local roster repair

Exit criteria:

- `AppCore::start_session` no longer creates local protocol invites
- a focused restore test proves persisted `SessionManager.local_invite` wins over a seed invite

### Milestone 5: Rename Pending Pairing State

Rename:

- `client` -> `pairing_client`
- `invite` -> `pairing_invite`
- `url` -> `pairing_url`

Exit criteria:

- unignore `completed_pairing_discards_pairing_invite_and_creates_stable_local_invite`
- pending state reads clearly as temporary pairing state

### Milestone 6: Extract Owner Resolution

This starts the broader protocol-engine contract.

Introduce an internal resolver returning an explicit enum:

```rust
enum OwnerResolution {
    Verified { owner: NdrOwnerPubkey },
    ProvisionalDeviceOwner { owner: NdrOwnerPubkey },
    PendingOwnerClaim {
        storage_owner: NdrOwnerPubkey,
        claimed_owner: NdrOwnerPubkey,
        device: NdrDevicePubkey,
    },
    UnknownDevice { device: NdrDevicePubkey },
    Conflict {
        device: NdrDevicePubkey,
        owners: Vec<NdrOwnerPubkey>,
    },
}
```

Start with invite observation:

- explicit owner hint
- ownerless known device
- ownerless unknown device
- owner conflict

Exit criteria:

- named unit tests for each owner-resolution branch
- no behavior change for legacy ownerless invites
- verified AppKeys owner always wins over fallback `owner=device`

### Milestone 7: Typed Missing Protocol State

Replace string targets like `owner:...` and debug-driven checks with:

```rust
enum MissingProtocolState {
    Roster { owner: NdrOwnerPubkey },
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

Exit criteria:

- tests assert missing requirements directly
- fetch-filter generation consumes typed missing requirements
- debug strings become observability, not the state API

### Milestone 8: Send Planning And Retry Cleanup

Extract a small send-planning layer that consumes:

- target owner
- roster knowledge
- invite/session availability
- payload

It should produce:

- prepared deliveries
- typed missing requirements

Then direct sends, local sibling sends, and group fanout can share the same missing-state model.

Exit criteria:

- smaller protocol send functions
- fewer ad hoc retry branches
- tests cover direct, local sibling, and group fanout missing-state cases with the same vocabulary

## Verification Commands

Default checks during the local-invite refactor:

```bash
cargo fmt --manifest-path core/Cargo.toml -- --check
cargo test --manifest-path core/Cargo.toml app_invite_consumers_use_protocol_owned_local_invite --no-run
cargo test --manifest-path core/Cargo.toml pending_linked_device_finishes_when_owner_accepts_invite
```

Intentional red test before implementation:

```bash
cargo test --manifest-path core/Cargo.toml app_invite_consumers_use_protocol_owned_local_invite -- --ignored
```

After each milestone, run the narrower affected tests first, then broaden to:

```bash
cargo test --manifest-path core/Cargo.toml protocol_filters_push
cargo test --manifest-path core/Cargo.toml app_keys_invites_requests
cargo test --manifest-path core/Cargo.toml protocol_runtime
```

Before merge, run the full core suite if time allows:

```bash
cargo test --manifest-path core/Cargo.toml
```

## Current Branch Status

Added tests:

- `core/src/core/tests/protocol_filters_push.rs`
  - ignored drift tests for app-facing invite consumers and identity publishing
- `core/src/core/tests/app_keys_invites_requests.rs`
  - ignored pairing-versus-stable-invite lifecycle test

Observed verification:

- `cargo fmt --manifest-path core/Cargo.toml -- --check` passes
- targeted compile for the ignored drift spec passes
- existing `pending_linked_device_finishes_when_owner_accepts_invite` passes
- running `app_invite_consumers_use_protocol_owned_local_invite -- --ignored` fails as expected because the current implementation still reads `LoggedInState.local_invite`

Local dependency note:

- the fresh Iris `origin/main` branch requires `nostr-double-ratchet` `0.0.146`
- `/Users/l/Projects/iris/nostr-double-ratchet` was clean and behind, so it was fast-forwarded to `origin/master` `3c0bdcd` to compile the tests
