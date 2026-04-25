# Architecture Implementation Plan

Implementation plan derived from [ARCHITECTURE.md](ARCHITECTURE.md) and the
current review in [ARCHITECTURE_REVIEW.md](ARCHITECTURE_REVIEW.md).

## Goal

Keep Iris Chat Rust-first:

- Rust owns navigation, app state, protocol/runtime behavior, persistence, and
  business rules.
- Android, iOS, and macOS stay thin native shells.
- The upstream `nostr-double-ratchet` runtime remains the protocol/runtime
  boundary; this app does not add a compatibility adapter layer around it.

## Current Status

Completed:

- migrated away from the vendored NDR crate
- wired the Rust core to upstream `NdrRuntime`
- kept upstream `FileStorageAdapter` as runtime state storage
- added and kept shell-contract coverage for Android and iOS
- split Android chat UI by responsibility
- split iOS chat UI into `ChatViews.swift`
- split Rust chat behavior into reactions, receipts, settings, typing, and the
  main chat flow

Still worth improving:

- split the remaining non-chat `ios/Sources/Views.swift` screens/components
- keep narrowing Rust modules when a behavior change naturally touches them
- broaden relay-backed iOS acceptance over time

## Invariants

Do not change these without a specific architecture decision:

- `AppAction` remains the input surface from native shells to Rust.
- `AppState` remains the Rust-owned render model.
- `Router` and `Screen` remain Rust-owned.
- Native shells restore secrets and persist secure side effects.
- Protocol/domain logic stays in Rust.
- Message history is app persistence, not native UI storage.

## Next Workstreams

### 1. Remaining Apple View Split

The chat surface has been split into `ios/Sources/ChatViews.swift`. Continue
splitting `ios/Sources/Views.swift` into focused files for settings, profile,
device roster, onboarding, and reusable controls as those areas change.

Rules:

- keep `AppManager.swift` as a thin Rust bridge
- avoid moving business decisions into SwiftUI views
- keep view files small enough to review visually and behaviorally

### 2. Rust Core Shape

The core is no longer a single-file hotspot. Future Rust splits should follow
active work:

- direct-message send/apply behavior
- group-message send/apply behavior
- attachment upload/download flow
- state projection and persistence when schema work changes

Avoid splitting just to create more modules.

### 3. Acceptance Coverage

Keep using:

```bash
just qa-native-contract
```

as the blocking refactor gate, and:

```bash
just qa-interop
```

as the heavier relay-backed confidence lane.

Broaden coverage later for:

- iOS to iOS relay-backed direct and group chat
- iOS restore-history convergence
- real-device acceptance beyond local emulator/simulator lanes

## Done Criteria

This plan is complete when:

- native shells remain thin and contract-tested
- runtime behavior is owned by Rust and upstream NDR
- no vendored or compatibility runtime layer returns
- large files are split around real responsibilities
- adding another shell primarily means rendering and platform integration
