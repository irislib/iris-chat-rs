# Changelog

## 0.1.12

- Create fresh one-use private invite links in the invite UI instead of exposing the relay-published local device invite secret.
- Register private invite response subscriptions immediately and include them in mobile push invite-response filters.
- Update to `nostr-double-ratchet` 0.0.136 for multi-invite runtime support and private invite owner-key handling.

## 0.1.11

- Update to `nostr-double-ratchet` 0.0.135, including the current direct
  subscription behavior, skipped-key sender coverage, and send-session
  selection parity.

## 0.1.10

- Update to `nostr-double-ratchet` 0.0.134, where low-level sessions only
  encrypt/decrypt unsigned inner events and chat/reaction/typing/expiration
  rumor construction lives in shared builder helpers.

## 0.1.9

- Update to `nostr-double-ratchet` 0.0.133 so Rust runtime/device-roster
  inspection includes stored device sessions even when AppKeys are not cached.

## 0.1.8

- Update to `nostr-double-ratchet` 0.0.132, including the shared runtime
  known-device roster helper and fresh same-owner send regression coverage.

## 0.1.7

- Keep restored same-secret CLI sessions from publishing a one-device AppKeys
  roster before relay backfill has merged existing devices.
- Make `iris sync --wait-ms` wait for protocol catch-up, and keep logged-in
  `iris relay set` from blocking before the new message-server list is saved.
- Add CLI interop coverage for a fresh same-secret client sending to a peer
  while an older session receives the message as its own outgoing sender copy.

## 0.1.6

- Update to `nostr-double-ratchet` 0.0.128 so TypeScript and Rust stacks share
  the same inactive send-capable session recovery release.

## 0.1.5

- Import matching legacy `ndr` filesystem ratchet sessions into Iris SQLite storage on account startup.
- Preserve active Iris ratchet state while filling missing or empty records from legacy storage.

## 0.1.4

- Make `iris listen` run the full Iris core/network listener and own the data-dir lock.
- Move read-only SQLite streaming to `iris tail --follow`.
- Add CLI interop coverage for receiving messages through `iris listen`.

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
