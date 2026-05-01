# Changelog

## 0.1.3

- Add `iris sync` for explicit CLI relay catch-up.
- Add group delete, member removal, admin add/remove, and group reaction commands.
- Add Iris CLI interop coverage against an independent `nostr-double-ratchet` protocol client.

## 0.1.2

- Add a per-data-dir core lock so only one writer/ratcheting Iris core can use a data directory at a time.
- Keep `iris listen`, `iris search`, and `iris tail` read-only so they can inspect SQLite without owning the core lock.

## 0.1.1

- Allow `iris send <user-id> ...` and related chat actions to accept direct user IDs without pre-creating a chat.
- Keep `iris send <npub> ...` compatible with the old `ndr send <npub> ...` agent workflow.

## 0.1.0

- Initial crates.io release of the `iris` command line client and `iris_chat_core` library.
