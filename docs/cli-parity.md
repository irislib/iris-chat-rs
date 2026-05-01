# CLI parity

`iris` is intended to replace the `ndr` CLI for normal agent and scripting
workflows while using the Iris shared app core and SQLite state.

## Covered workflows

- Account create, restore, logout, and identity display.
- JSON output and explicit data directory selection.
- Direct chat create/open/list/delete.
- Send, read, search, tail, and long-running listen.
- Disappearing messages with `--ttl` or `--expires-at`.
- Reactions, typing indicators, seen receipts, and delivered receipts.
- Public invite create/accept.
- Device link invite create/accept.
- Group create/list/show/send/read/add/rename.
- Message server list/add/remove/set/reset.
- SQLite-backed process restart persistence.

## Not carried over from ndr

- `receive <raw-event-json>`: Iris processes relay events through the app core
  instead of exposing a raw decrypt command.
- Contact petnames: Iris uses profile and chat state rather than a separate CLI
  contact book.
- Nearby peer management: nearby remains an app transport feature, not a
  published CLI management surface.
- Cross-language `ndr` protocol interop tests remain in `nostr-double-ratchet`.
  Iris CLI coverage focuses on the app-core CLI workflows.

## Test coverage

`core/tests/cli.rs` runs the published CLI shape as real subprocesses against
temporary data directories. It covers account persistence across processes,
offline direct messaging, reactions, TTL messages, typing command plumbing,
read/search/tail, long-running `listen`, invites, groups, relay configuration,
link invite creation, and logout.
