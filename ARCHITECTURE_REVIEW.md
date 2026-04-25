# Architecture Review

Rust-first review of the Iris Chat app as of 2026-04-26.

## Executive Summary

The app architecture is in the right shape after the `nostr-double-ratchet`
migration. The repo uses the upstream runtime through the Rust core, native
shells remain render/bridge layers, and there is no vendored NDR crate left in
this repo.

The main cleanup needed from the migration was not an adapter issue. It was
file shape: chat UI and chat-domain behavior had grown into mixed-responsibility
files. Android chat rendering, iOS chat rendering, and Rust chat behavior are
now split by concern.

## Current Boundary

Rust owns:

- `AppAction`, `AppState`, `AppUpdate`, routing, and durable app state
- `NdrRuntime` setup, relay/runtime event handling, direct chats, group chats,
  reactions, receipts, typing indicators, disappearing-message settings, and
  persistence
- upstream NDR runtime storage through `FileStorageAdapter`

Native owns:

- rendering and shell lifecycle
- secure credential storage primitives
- narrow platform effects such as clipboard, document picking, file opening,
  notifications, and sharing
- ephemeral presentation state such as drafts, focus, scroll, and menus

## Migration Leftovers

No app-side runtime wrapper or compatibility adapter needs removal.

- `core/Cargo.toml` points at the sibling upstream `nostr-double-ratchet` crate.
- `core/src/core/account.rs` constructs `NdrRuntime` directly.
- `FileStorageAdapter` is an upstream runtime storage implementation used for
  group and ratchet state. It is not message-history storage and not a native
  app abstraction leak.
- Legacy migration code that remains is data-preserving app migration logic,
  such as default relay migration and secure-secret restore tests. It should
  stay unless a schema reset is intentionally chosen.

## Cleanup Done

Android chat UI is now separated into:

- `ChatScreen.kt` for screen orchestration, timeline state, dispatch, and top
  level layout
- `ChatMessageComponents.kt` for message rows, reactions, reply parsing, links,
  and clustering
- `ChatComposer.kt` for draft, emoji, reply composer, and send controls
- `ChatAttachments.kt` for attachment picking, previews, cache copies, and file
  opening

Rust chat behavior is now separated into:

- `chats.rs` for chat creation/opening, message send/apply flow, and shared
  thread mutation
- `chat_reactions.rs`
- `chat_receipts.rs`
- `chat_settings.rs`
- `chat_typing.rs`

iOS chat UI is now separated into:

- `Views.swift` for root navigation and non-chat screens
- `ChatViews.swift` for the chat timeline, message rows, reactions, reply
  parsing, attachments, and image viewer

## Remaining Debt

The largest remaining maintainability issue is still Apple shell file shape,
but it is narrower than before:

- `ios/Sources/Views.swift` still contains several non-chat screens and shared
  helper views.
- `ios/Sources/IrisChrome.swift` is still a broad shared UI component file.

The next Rust-core cleanup, if needed, should be incremental rather than a
boundary redesign:

- split direct-message and group-message sending further only when changing
  that behavior
- keep projection and persistence modules small as schema work continues
- keep full-state snapshots until profiling shows they are a bottleneck

## Bottom Line

The architecture does not need an adapter route or legacy compatibility layer.
Keep the clean Rust-owned runtime boundary, keep native shells thin, and keep
splitting large files along behavior boundaries as they become active work.
