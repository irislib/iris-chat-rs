# Changelog

## 0.3.0

- Require mutable access for `ProtocolEngine::exit_batch` so consumed delivery
  markers retire only after the checkpoint is successfully saved. Failed writes
  retain pending work for retry. This is a breaking Rust API change.
- Consume authenticated own-device edit and deletion controls before plaintext
  delivery journaling. Applications synchronize these controls through their
  history-aware reconciliation path; remote participant delivery is preserved.
- Bound group decryption per turn, preserve new sender-key wakeups during an
  existing search, and discard prepared work for revoked candidates.
- Borrow recipient device lists when sending and retrying rather than copying
  unrelated sessions and saved keys.
- Preserve existing checkpoints, sessions, keys, and the wire format.

## 0.2.1

- Reject linked-device group copies whose protocol conflicts with authenticated
  group history. Preserve valid metadata changes, removals, signed protocol
  changes, and recovery of a previously corrupted local copy.

## 0.2.0

- Replace ambiguous sender-and-second acknowledgements with bounded, durable
  exact-ciphertext tracking. Replayed group messages no longer trigger repeated
  decryption and repair requests after restart or event-cache eviction.
- Preserve repair backoff for pending messages and retry decryption when group
  metadata or sender keys change. Distinct messages from the same second remain
  eligible for recovery.
- Remove `acknowledge_delivered_group_sender_key_message`; callers no longer
  need to acknowledge group ciphertext separately. This is a breaking API change.
- Read existing checkpoints without migrating keys, sessions, or messages.

## 0.1.9

- Let a runtime holding the profile secret key send immediately while its
  signed linked-device roster is still being recovered.

## 0.1.8

- Require signed AppKeys evidence before accepting an invite's claimed owner
  and device, including retryable blocks while the owner roster is missing.
- Bound pending group-fanout retries and retain their recovery state across
  restarts.
- Align the published crate with `nostr-double-ratchet` 0.0.164.
