# State Refactoring Design: Local Invite Ownership

Branch: `state-refactoring`

Base analyzed: `origin/main` at `ce5cb7f`

## Goal

Move toward a simpler ownership model for login/session state without changing protocol behavior:

- `PendingLinkedDeviceState` may own a temporary pairing invite before the app is logged in.
- Once an account session exists, the stable local NDR invite should be owned by `SessionManager`, reached through `ProtocolEngine`.
- `LoggedInState` should not own protocol invite material.

This document is based on static analysis only. It intentionally does not make the refactor yet.

## Current State Shape

`AppCore` currently has four relevant top-level state holders:

- `logged_in: Option<LoggedInState>`
- `protocol_engine: Option<ProtocolEngine>`
- `pending_linked_device: Option<PendingLinkedDeviceState>`
- `private_chat_invites: BTreeMap<String, Invite>`

`LoggedInState` currently mixes identity, relay runtime, and protocol bootstrap material:

```rust
pub(super) struct LoggedInState {
    pub(super) owner_pubkey: PublicKey,
    pub(super) owner_keys: Option<Keys>,
    pub(super) device_keys: Keys,
    pub(super) client: Client,
    pub(super) relay_urls: Vec<RelayUrl>,
    pub(super) local_invite: Invite,
    pub(super) authorization_state: LocalAuthorizationState,
}
```

`PendingLinkedDeviceState` is the temporary pre-login pairing state:

```rust
pub(super) struct PendingLinkedDeviceState {
    pub(super) device_keys: Keys,
    pub(super) client: Client,
    pub(super) invite: Invite,
    pub(super) url: String,
}
```

The important distinction is that `PendingLinkedDeviceState.invite` is a temporary pairing invite, while `LoggedInState.local_invite` is intended to be the stable local device invite. Those two concepts currently share the same type and similar names.

## Current Lifecycle

### Pairing Before Login

`StartLinkedDevice` calls `create_pending_linked_device`.

That function:

- stops any previous pending link flow
- generates fresh `device_keys`
- creates an ownerless invite with `purpose = "link"` and `max_uses = Some(1)`
- creates a temporary `Client` from the new device keys
- subscribes for `INVITE_RESPONSE_KIND` events addressed to the invite's ephemeral pubkey
- stores the result in `PendingLinkedDeviceState`

`handle_pending_link_device_response` later:

- checks for `INVITE_RESPONSE_KIND`
- decrypts/processes the event against `pending.invite`
- extracts `owner_pubkey`, peer device id, and NDR session state
- calls `complete_pending_linked_device`

`complete_pending_linked_device`:

- stops the temporary pending client
- calls `start_session(owner_pubkey, None, device_keys, false, false)`
- imports the received session state into `ProtocolEngine`
- refreshes subscriptions and fetches protocol state

This lifecycle is correctly pre-login. There is no owner account and no `ProtocolEngine` yet, so the pending state needs to hold its own temporary invite.

### Login / Restore

`start_session` is the current account-session constructor. It:

- clears existing runtime state
- optionally restores persisted app state
- builds a `SqliteStorageAdapter` scoped by owner/device
- loads private chat invites
- calls `load_or_create_local_invite`
- creates seed `SessionManager` and `NostrGroupManager` snapshots
- calls `ProtocolEngine::load_or_seed(..., local_invite.clone(), ...)`
- stores the same `local_invite` into `LoggedInState`
- starts the Nostr client and network/subscription machinery

This is where the stable local invite becomes duplicated.

### Protocol Engine Load

`ProtocolEngine::load_or_seed` receives `local_invite` from `AppCore`.

It loads the persisted protocol state if possible. Then it does:

```rust
if engine.session_manager.snapshot().local_invite.is_none() {
    engine.session_manager.replace_local_invite(local_invite.clone());
}
engine.ensure_local_roster(local_invite.created_at);
```

So `SessionManager` already has the correct conceptual ownership. However, `AppCore` keeps a second copy in `LoggedInState`.

One subtle concern: if the persisted `SessionManager` already contains a local invite, `load_or_seed` keeps that persisted invite but still calls `ensure_local_roster` with the seed invite's `created_at`. Under normal storage this should match, but the function contract does not enforce that.

## Current Stable Invite Consumers

These app-layer consumers read `LoggedInState.local_invite` directly:

- `projection.rs`
  - `build_public_invite_snapshot`
  - `build_link_device_snapshot`
- `publishing.rs`
  - `publish_local_identity_artifacts`
- `protocol.rs`
  - `protocol_invite_response_pubkeys`
- `mobile_push.rs`
  - `build_mobile_push_sync_snapshot`
- tests and test helpers

These protocol-layer consumers read `SessionManager.local_invite`:

- `ProtocolEngine::observe_invite_response_event`
- protocol tests through `local_invite_for_test`

The duplicated source of truth is the main design problem.

## Desired Ownership

The target model should be:

```text
Before pairing completes:
  PendingLinkedDeviceState
    owns temporary ownerless link invite
    owns temporary client listening for invite response

After login/session creation:
  ProtocolEngine
    SessionManager
      owns stable owner-bound local invite

  LoggedInState
    owns account/device identity and authorization state
    does not own local invite
```

The pending pairing invite should not become the stable local invite. It has different semantics:

- `purpose = "link"`
- ownerless at creation time
- one-use bootstrap material
- exists before the account owner is known

The stable local invite should be owner-bound protocol identity:

- `owner_public_key = Some(owner_pubkey)`
- device identity matches the current device
- lives in protocol/session state
- is used for normal NDR session discovery, invite responses, push filters, and local identity publishing

## Proposed Refactor

### Step 1: Add Explicit Protocol Accessors

Add non-test accessors to `ProtocolEngine`:

```rust
impl ProtocolEngine {
    pub(super) fn local_invite(&self) -> Option<Invite>;
    pub(super) fn local_invite_response_pubkey(&self) -> Option<PublicKey>;
}
```

Optionally add app-level helpers to reduce repeated `Option` plumbing:

```rust
impl AppCore {
    fn local_protocol_invite(&self) -> Option<Invite>;
    fn local_protocol_invite_response_pubkey(&self) -> Option<PublicKey>;
}
```

The first refactor pass can leave `load_or_create_local_invite` in `account.rs`, but all read paths should switch from `logged_in.local_invite` to the protocol accessor.

### Step 2: Update Stable Invite Consumers

Change these call sites to read from `ProtocolEngine`:

- public invite snapshot fallback
- linked-device "awaiting approval" snapshot
- local identity artifact publishing
- protocol invite-response filters
- mobile-push invite-response pubkeys

For `publish_local_identity_artifacts`, only the invite event depends on the local invite. Profile metadata and AppKeys publishing can still proceed if the protocol engine is absent.

For projections and mobile push, if `logged_in` exists but `protocol_engine` is missing, prefer returning no local invite data rather than reading a stale login copy. A missing protocol engine during a logged-in state is already an unhealthy transitional state.

### Step 3: Remove `LoggedInState.local_invite`

Once all consumers are routed through `ProtocolEngine`, delete the field from `LoggedInState`.

This gives a compile-time guard against new app code reintroducing the old ownership boundary.

### Step 4: Move Local Invite Creation Into Protocol Ownership

The deeper cleanup is to move `load_or_create_local_invite` out of `account.rs`.

Possible shape:

```rust
impl ProtocolEngine {
    pub(super) fn load_or_seed_for_local_device(
        storage: Arc<dyn StorageAdapter>,
        owner_pubkey: PublicKey,
        device_keys: &Keys,
        seed_session_manager: SessionManagerSnapshot,
        seed_group_manager: GroupManagerSnapshot,
    ) -> anyhow::Result<Self>;
}
```

That method would:

- derive the device id from `device_keys.public_key()`
- load or create the stable owner-bound invite
- seed `SessionManager.local_invite` if missing
- ensure the local roster using the actual invite in `SessionManager`

This would make `AppCore::start_session` stop knowing how local protocol invites are stored.

If that feels too much for `ProtocolEngine`, create a small `LocalProtocolIdentityStore` helper near protocol-engine code rather than keeping it in account-login code.

### Step 5: Rename Pending Pairing Fields

After stable invite ownership is fixed, rename the pending state to make the two invite classes obvious:

```rust
pub(super) struct PendingLinkedDeviceState {
    pub(super) device_keys: Keys,
    pub(super) pairing_client: Client,
    pub(super) pairing_invite: Invite,
    pub(super) pairing_url: String,
}
```

This is not necessary for correctness, but it removes ambiguity from future code reviews.

## Test Coverage Today

Current tests cover several important behaviors:

| Behavior | Existing coverage |
| --- | --- |
| Pending linked device creates an ownerless link invite | `start_linked_device_creates_ownerless_link_invite` |
| Pending linked device completes when owner accepts | `pending_linked_device_finishes_when_owner_accepts_invite` |
| Protocol engine stores a local invite | protocol tests using `local_invite_for_test` |
| Invite-response backfill includes local invite pubkey | `recent_protocol_filters_include_runtime_invite_response_backfill` |
| Live invite-response filters include peer device authors | `protocol_filters_track_invite_responses_by_known_device_authors` |
| Private invite does not reuse stable local invite secret | `create_invite_generates_private_link_without_public_republish` |
| Mobile push includes local invite response pubkey | `mobile_push_snapshot_tracks_local_invite_when_enabled` |
| Mobile push includes private invite response pubkeys | `mobile_push_snapshot_tracks_private_invite_when_enabled` |
| Mobile push can omit invite response pubkeys when disabled | `mobile_push_snapshot_omits_local_invite_when_disabled` |

This is good behavioral coverage around invite usage, but it does not directly cover the ownership refactor.

## Test Gaps For This Refactor

### 1. No Drift Test Between Login Copy And SessionManager Copy

Most app-layer tests currently read or construct `LoggedInState.local_invite`. The suite does not intentionally create a different invite inside `SessionManager` and then assert that app projections, mobile push, protocol filters, and identity publishing use the protocol-owned invite.

That is the exact failure mode this refactor is meant to prevent.

Add tests where the app has a logged-in account and protocol engine invite B, while any legacy login invite A is different. During the transition this can be done with hand-built state. After field removal, the compiler enforces that only B exists.

Test expected outputs:

- `build_public_invite_snapshot` uses protocol invite B when there is no private invite.
- `build_link_device_snapshot` uses protocol invite B for linked devices awaiting approval.
- `build_mobile_push_sync_snapshot` includes B's invite response pubkey.
- `recent_protocol_filters` includes B's invite response pubkey.
- `publish_local_identity_artifacts` publishes B as the invite event.

### 2. Pairing Invite Versus Stable Invite Is Not Explicitly Tested

`pending_linked_device_finishes_when_owner_accepts_invite` verifies that pairing completes and imports a session, but it does not assert that the temporary link invite is discarded and a stable owner-bound local invite is created under `SessionManager`.

Add a test:

- start linked-device flow
- capture `pending.pairing_invite`
- accept it
- complete pairing through relay event handling
- read `protocol_engine.local_invite`
- assert:
  - stable invite has `owner_public_key = Some(owner)`
  - stable invite does not have `purpose = Some("link")`
  - stable invite does not have `max_uses = Some(1)`
  - pending state is gone

The stable invite may or may not share the device pubkey with the pairing invite. It should share the device identity but not the pairing-only semantics.

### 3. Protocol Engine Restore Semantics Need A Focused Test

`ProtocolEngine::load_or_seed` should be tested for this scenario:

- storage already contains a persisted `SessionManager` with local invite A
- caller passes seed invite B
- loaded engine keeps A
- local roster repair uses A's timestamp/identity, not B's

This matters because moving invite creation into protocol ownership should also clarify which invite is authoritative during restore.

### 4. Test Helpers Encode The Old Ownership

Several tests manually construct `LoggedInState { local_invite: invite, ... }` and then install a protocol engine using that invite.

That pattern makes it easy for tests to keep passing while production code has duplicated state.

Recommended helper changes:

- centralize app-core test setup behind `logged_in_test_core`
- make test setup install `ProtocolEngine` first
- remove any need for tests to manually pass a local invite into `LoggedInState`
- expose a test-only helper for reading the protocol-owned invite where assertions need it

The refactor should intentionally break direct `LoggedInState` construction sites and migrate them to helpers.

### 5. Missing Protocol Engine Transitional Behavior

Some current tests manually create `logged_in` without a protocol engine. After the refactor, invite consumers should handle this explicitly:

- projection returns no public invite fallback
- mobile push omits local invite response pubkey
- identity publishing skips invite event but can still publish profile/AppKeys if possible

Add one or two narrow tests so this behavior is deliberate.

## Refactor Risks

### Lost Invite Publishing

If `publish_local_identity_artifacts` is changed to require `protocol_engine` too early, profile metadata or AppKeys publishing may accidentally stop when only the invite is unavailable.

Mitigation: fetch the invite separately and gate only the invite event on it.

### Empty Push / Subscription Filters

`protocol_invite_response_pubkeys` and mobile push rely on the local invite response pubkey. If the protocol engine accessor is missing or returns `None`, invite responses may not be fetched or pushed.

Mitigation: add focused tests for both recent filters and mobile push snapshots reading the protocol-owned invite.

### Confusing Private Invite Behavior

Private one-use invites in `private_chat_invites` are separate from the stable local protocol invite. Do not move them into `SessionManager` as part of this refactor.

Mitigation: keep `create_invite_generates_private_link_without_public_republish` and make it compare against the protocol-owned stable invite.

### Overloading The Pending Pairing Invite

The pending link invite should not be installed as the stable local invite after pairing. It is bootstrap material, not regular protocol identity.

Mitigation: add the pairing-versus-stable test described above and rename pending fields.

## Suggested Implementation Order

1. Add `ProtocolEngine::local_invite` and `ProtocolEngine::local_invite_response_pubkey`.
2. Add or update tests so app-facing invite consumers assert against the protocol-owned invite.
3. Update app consumers to read from `ProtocolEngine`.
4. Remove `LoggedInState.local_invite`.
5. Update test helpers and direct `LoggedInState` constructions.
6. Move `load_or_create_local_invite` into protocol-engine ownership.
7. Rename pending pairing fields for clarity.

Steps 1-5 are the minimal boundary fix. Step 6 is the deeper ownership cleanup. Step 7 is readability.

## Acceptance Criteria

- `LoggedInState` no longer has a `local_invite` field.
- Stable local invite reads go through `ProtocolEngine` / `SessionManager`.
- Pending linked-device flow still creates an ownerless temporary link invite before login.
- Completing pending pairing produces a linked account session and imports the NDR session state.
- The stable local invite after login is owner-bound and protocol-owned.
- Public invite snapshot fallback, link-device snapshot, identity publishing, protocol filters, and mobile push snapshots all use the protocol-owned stable invite.
- Direct tests no longer manually synchronize a login invite with a protocol-engine invite.
