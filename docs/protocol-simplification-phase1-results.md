# Protocol Simplification Phase 1 Results

## Summary

Phase 1 hard-deleted procedural protocol backfill and retry loops while keeping retry for already-signed relay publishes. The resulting model is intentionally subtractive:

- protocol calls either produce immediate signed publish effects or no side effects
- missing protocol state does not enqueue protocol fetches, repairs, fanouts, or replay work
- relay subscriptions are still derived from durable protocol/app state
- `pending_relay_publishes` remains the durable retry queue for already-signed events
- `pending_decrypted_deliveries` remains only as app persistence acknowledgment bookkeeping

This phase does not add the replacement readiness model. The ignored and adjusted tests below are the concrete readiness gaps for Phase 2.

## Removed Production Mechanisms

- Removed `ProtocolEffect::FetchProtocolState`; protocol effects now only carry already-prepared Nostr events to publish.
- Removed `queued_targets` from protocol result structs and call sites.
- Removed queued protocol target filter generation and queued-target fetch handling.
- Removed persisted pending protocol retry state for:
  - outbound direct messages
  - inbound replay
  - group fanout retries
  - group pairwise payload retries
  - sender-key repair retries
  - sender-key candidate queues
- Removed core internal events used only for procedural catch-up:
  - `FetchCatchUpEvents`
  - `FetchTrackedPeerCatchUp`
  - `SyncComplete`
  - `ProtocolAuthorBackfillComplete`
- Removed protocol fetch/backfill runtime counters and in-flight tracking.
- Removed procedural fetch helpers:
  - `fetch_recent_protocol_state`
  - `fetch_recent_protocol_metadata_state`
  - `fetch_protocol_state_for_filters`
  - `fetch_recent_messages_for_tracked_peers`
  - protocol author backfill scheduling
- Removed liveness-driven protocol retry scheduling. Liveness now only reconciles subscriptions and retries signed relay publishes.
- Removed sender-key repair loops for missing distributions/revisions. Immediate repair responses to valid incoming repair requests are still allowed.

## Preserved Mechanisms

- `pending_relay_publishes`
- `retry_pending_relay_publishes`
- relay publish liveness checks
- state-derived subscription refresh on login, foreground, reconnect, relay changes, and protocol state changes
- immediate protocol publish effects from successful direct/group work
- local persistence acknowledgment through `pending_decrypted_deliveries`

## Test Changes

### Deleted Or Excluded Obsolete Behavior

- `chat-protocol/src/protocol_engine.rs`
  - Removed local unit tests for queued targets and protocol discovery fetch effects:
    - `local_sibling_roster_probe_does_not_block_delivery`
    - `known_local_sibling_target_blocks_delivery`
    - `missing_remote_roster_blocks_delivery`
    - `protocol_discovery_effects_fetch_appkeys_and_invites_for_owner`
- `core/src/core/tests/protocol_runtime.rs`
  - Deleted. These tests exercised catch-up fetches, queued targets, and pending protocol retry scheduling.
- `core/src/core/tests/protocol_runtime_replay.rs`
  - Deleted. These tests exercised pending inbound/outbound replay queues.
- `core/src/core/tests/protocol_filters_push.rs`
  - Deleted. These tests exercised queued-target filter behavior.
- `core/src/core/tests/first_contact_receiver.rs`
  - Deleted. These tests depended on bootstrap backfill for first-contact receive.
- `core/src/core/tests/groups_sender_key_retry.rs`
  - Deleted. These tests exercised sender-key repair retry loops.
- `core/src/core/tests/direct_group_sender_key_ack.rs`
  - Deleted. It only covered sender-key candidate queue cleanup after app persistence acknowledgment.
- `core/src/core/tests/groups_sender_key.rs`
  - Deleted tests whose purpose was pending sender-key replay or repair:
    - `appcore_sender_key_outer_before_distribution_retries_after_control_state`
    - `appcore_sender_key_missing_rotated_distribution_repairs_and_applies_pending_outer`
    - `appcore_sender_key_repair_response_survives_sender_restart`
    - `appcore_sender_key_mixed_order_storm_converges`
    - `appcore_sender_key_late_member_repair_denies_pre_join_outer`
    - `appcore_sender_key_late_member_repair_allows_post_join_missed_distribution`
    - `appcore_sender_key_pending_outer_survives_restart_and_applies_once`

### Still-Relevant Tests Adjusted

- `core/src/core/tests.rs`
  - Removed obsolete test includes, updated persisted protocol fixtures to omit deleted pending queues, and kept shared helpers needed by surviving tests.
- `core/src/core/tests/retry_publish_ordering.rs`
  - Removed `queued_targets` from the constructed retry result. The test still covers signed relay publish retry ordering.
- `core/src/core/tests/direct_messages_typing.rs`
  - Updated opening an uncached direct chat to assert it no longer starts catch-up protocol fetch work.
- `core/src/core/tests/app_keys_invites_requests.rs`
  - Replaced recent backfill filter assertions with current state-derived subscription filter assertions.
- `core/tests/cli.rs`
  - Updated offline direct send from `queued` to `sent` because Phase 1 no longer has a protocol retry queue for unknown first-contact sends. This is a readiness semantics gap.

### New Readiness Gaps Recorded

- `core/tests/cli_interop.rs`
  - Marked ignored until Phase 2 readiness work:
    - `iris_listen_receives_first_contact_sent_to_user_id`
      - Direct chat by user id no longer auto-discovers protocol state through first-contact backfill.
    - `restored_same_nsec_cli_send_reaches_peer_and_self_syncs_to_existing_session`
      - Restored account/device no longer uses startup backfill to merge existing protocol state before sending.
    - `sender_key_cli_group_interop_three_members_restart_and_restored_owner_device`
      - Group sender-key restart/restored-device recovery no longer has repair/backfill loops.

## Phase 2 Readiness Gaps

- Account readiness: restored accounts need an explicit state for "protocol state not yet current enough to publish app keys or send."
- Direct chat readiness: a direct chat created from only a user id/npub needs to be not ready until app keys/session state exists through subscription-derived state.
- Group readiness: a group needs to be not ready until membership metadata and required sender-key author state are available.
- Delivery semantics: sends that cannot produce protocol publish effects should not look successfully sent just because the local app row exists.
- Recovery semantics: foreground/reconnect must rely on subscription reconciliation and durable state, or Phase 2 must prove a narrow targeted fetch is required.

## Verification

Commands run:

```sh
cd /Users/l/Projects/iris-core-architecture/iris-chat-rs/chat-protocol
cargo test
```

Result: passed, `10 passed; 0 failed`.

```sh
cd /Users/l/Projects/iris-core-architecture/iris-chat-rs/core
cargo test
```

Result: passed. Core suite: `225 passed; 0 failed; 3 ignored`; CLI interop: `4 passed; 0 failed; 3 ignored`; all remaining integration and doc tests passed.
