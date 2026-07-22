# Changelog

## 0.1.9

- Let a runtime holding the profile secret key send immediately while its
  signed linked-device roster is still being recovered.

## 0.1.8

- Require signed AppKeys evidence before accepting an invite's claimed owner
  and device, including retryable blocks while the owner roster is missing.
- Bound pending group-fanout retries and retain their recovery state across
  restarts.
- Align the published crate with `nostr-double-ratchet` 0.0.164.
