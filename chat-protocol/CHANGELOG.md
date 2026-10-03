# Changelog

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
