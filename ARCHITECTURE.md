# Architecture

This document is the canonical source of truth for the Iris Chat architecture.
When the ownership boundary changes, update this document in the same branch.

The repo follows a Rust-first mobile architecture.

## Goal

Keep as much product behavior as possible in Rust so that:

- app state is cross-platform
- navigation is cross-platform
- business logic is cross-platform
- protocol and relay behavior are cross-platform
- adding another native shell is mostly a rendering and platform-integration task

Android, iOS, and macOS are intentionally thin shells over one shared Rust app
core.

## Repo Shape

- `core/`
  - shared Rust mobile app core
  - UniFFI boundary
  - router, app state, protocol/runtime logic, persistence, and support/debug data
- `android/`
  - Android shell
  - Compose rendering
  - Android secure storage and platform integrations
- `ios/`
  - iOS shell
  - SwiftUI rendering
  - Keychain and iOS platform integrations
- `scripts/`
  - local build, test, harness, and release entrypoints
- `tools/`
  - higher-signal local run and doctor commands

The protocol runtime is an external dependency. The product boundary for this
app is the `core/` crate in this repo.

## Core Principles

### Rust owns app truth

Rust is the authoritative owner of:

- routing and navigation state
- account and session state
- device authorization state
- direct-chat and group-chat behavior
- relay/protocol behavior
- retry, catch-up, and background messaging policy
- user-visible long-running operation state
- durable application persistence

Native code must not introduce alternate correctness rules for those areas.

### Native shells render Rust state

Android and iOS should primarily:

- construct the Rust core
- restore secure credentials into Rust at startup
- persist Rust-emitted secure side effects
- render Rust `AppState`
- dispatch user actions back to Rust
- host real platform integrations such as camera, clipboard, and sharing

### Rust-first does not mean Rust-only at any cost

If a platform API is required for first-class UX, native code may host a narrow
capability bridge. Rust still owns:

- policy
- state transitions
- retries and fallbacks
- user-visible outcomes

Native executes platform effects. It does not fork app logic.

## Ownership Split

### Rust owns

- `FfiApp` and the UniFFI boundary
- `AppAction`, `AppState`, `AppUpdate`
- `Router` and `Screen`
- account creation and restore rules
- linked-device authorization rules
- chat/group state and mutations
- relay session configuration
- subscription planning and catch-up
- relay-event interpretation
- outbound/inbound message lifecycle
- persistence schema and migration
- support bundle and runtime debug projection

### Native owns

- rendering and UI composition
- secure secret storage primitives
  - Android Keystore and DataStore
  - iOS Keychain
- platform lifecycle hooks
- clipboard, share sheet, camera, and QR host APIs
- shell-only ephemeral UI state
  - text input drafts
  - focus
  - scroll position
  - local presentation toggles

### Native must not own

- message transport policy
- protocol interpretation
- alternate routing logic
- synthetic authoritative app state
- domain-specific retry rules
- authorization or chat/group business rules

## Data Flow

The app uses a one-way Rust-owned state flow:

1. Native constructs `FfiApp`.
2. Native calls `state()` once for the initial snapshot.
3. Native starts listening for `AppUpdate`.
4. User interactions dispatch `AppAction` into Rust.
5. Rust mutates internal actor state.
6. Rust emits a new `AppUpdate`.
7. Native applies the update and re-renders.

The Rust actor is the only authoritative owner of app state transitions.

## State Model

`AppState` is the UI-facing snapshot sent to native shells. It intentionally
contains:

- `router`
- account/device authorization slices
- chat list and current chat slices
- group details
- busy/in-flight flags
- toast and other user-visible transient state

Rust may keep additional internal bookkeeping that is not exposed directly in
`AppState`. Native renders the projection, not the internal storage.

Current relevant files:

- `core/src/actions.rs`
- `core/src/state.rs`
- `core/src/updates.rs`

## Router Model

Routing is Rust-owned.

`AppState.router` is the authoritative navigation model:

- `default_screen`
- `screen_stack`

Rust decides:

- what the current logical route is
- when business actions move the user to another screen
- what render slice must exist for a route

Native decides only how to present that route with platform UI frameworks.

Invariant:

- when the top route points at a screen that needs data, Rust is responsible for
  keeping the corresponding `AppState` slice populated

Example:

- if the top route is `Screen::Chat { chat_id }`, Rust keeps `current_chat`
  consistent with that chat

Native should not fetch screen data on its own.

## Update Model

The current boundary uses:

- `AppUpdate::FullState(AppState)`
- side-effect updates for secure credential persistence

Each full-state snapshot carries a monotonic `rev`.

Native keeps `lastRevApplied` and drops stale `FullState` snapshots. Side-effect
updates that persist secure material must still be applied even when their `rev`
is older than the latest full snapshot.

This simple model remains acceptable until profiling shows that FFI transfer
size or snapshot cloning is a real bottleneck.

## Persistence Model

There are two persistence layers.

### Native secure storage

Native stores only secret/auth material that should not live in the Rust
plaintext app snapshot.

Examples:

- owner/device secret bundle
- future signer/session credentials if needed

### Rust app persistence

Rust stores the rest of the durable app model.

Examples:

- upstream NDR runtime, group, and ratchet state
- thread/message model
- pending outbound state
- owner profile cache
- relay-event de-duplication metadata

Rust owns the format, versioning, and migration of that persisted state.

## Native Shell Contract

The shell contract is:

1. Build the Rust object.
2. Load secure credentials from native secure storage.
3. Dispatch a restore action into Rust if credentials exist.
4. Persist secure side-effect updates emitted by Rust.
5. Render Rust state.
6. Forward user actions and lifecycle signals into Rust.

Two rules matter here:

- Rust owns authoritative state and routing.
- Native shells may not synthesize app truth to paper over Rust behavior.

Current shell implementations:

- Android: `android/app/src/main/java/to/iris/chat/core/AppManager.kt`
- iOS: `ios/Sources/AppManager.swift`

## Verification Gates

The architecture now has explicit boundary tests:

- Android shell contract suite:
  - `android/app/src/androidTest/java/to/iris/chat/core/AppManagerContractTest.kt`
- iOS shell contract and unit suite:
  - `ios/Tests/IrisChatTests.swift`
- Android local UI smoke:
  - `android/app/src/androidTest/java/to/iris/chat/IrisChatUiSmokeTest.kt`
- iOS local UI smoke:
  - `ios/UITests/IrisChatUITests.swift`

Blocking refactor gate:

- `just qa-native-contract`

Heavier non-blocking confidence lane:

- `just qa-interop`

## Current Refactor Focus

The architecture direction is correct. The app should keep the upstream NDR
runtime boundary and keep native shells thin.

The current cleanup focus is file shape, not a boundary redesign. Split large
files by real responsibility when touching them, especially the remaining broad
iOS SwiftUI views and any Rust modules that grow around active behavior changes.
