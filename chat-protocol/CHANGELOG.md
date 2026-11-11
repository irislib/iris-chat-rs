# Changelog

## Unreleased

- Stop naming the recipient device on message events. Kind 1060 envelopes now
  carry only the version and header tags (nostr-double-ratchet 0.0.177), so a
  relay can no longer link deliveries, or a group fanout burst, to the devices
  receiving them. Invite responses still name only the public invite key.
- Record what each publish effect carries in `ProtocolPublish::carrier`: a
  pairwise envelope with its target device, or shared group ciphertext. The two
  now look alike on the wire, so hosts must route and filter by this record
  (`ProtocolPublish::is_publishable`), never by tags. Shared group ciphertext
  still never becomes a publication.
- Keep reading recipient-tagged envelopes from older clients, including
  dropping ones that name another device. Never offer them for publication:
  ready publications sealed by an older version are retired.

## 0.3.16

- Report authenticated group key distributions that are still waiting for
  their group's metadata, and let an existing member resend that metadata
  privately to another member.
- Avoid allocating the outgoing backlog when loading state for a notification
  preview.
- Build the engine and its tests as ordinary modules. No wire or storage
  format changes.

## 0.3.15

- Deliver Note to Self to each reachable linked device. Retry missing copies across restarts while honoring device removals.

## 0.3.14

- Expose exact signed invite-owner device authorization to notification previews,
  including encrypted proofs ingested through the read-only preview path.
- Avoid rewriting checkpoints when remote-device and linked-device retries only
  advance their retry clocks. Persist session, target, and delivery changes while
  preserving save-failure rollback and recovery after restart.

## 0.3.3

- Retry queued sender-key controls immediately when authenticated group metadata
  arrives from a linked device or a roster snapshot. Keep unchanged metadata
  backoff, sender authorization, and sibling forwarding guards.

## 0.3.1

- Compare SQLite checkpoints with one reusable 4 KiB buffer instead of copying
  the previous value. Preserve TEXT compatibility, changed-page writes, namespace
  boundaries, atomic rollback, and existing checkpoints and sessions.

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
