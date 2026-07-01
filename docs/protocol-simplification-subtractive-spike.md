# Protocol Simplification Subtractive Spike

## Summary

This plan describes a diagnostic cleanup pass for the protocol runtime. The goal is to remove hidden retry and backfill loops first, observe what compile and test failures reveal, and then write the replacement architecture from those facts.

The target model is:

- FFI actions and relay events enter one serialized core path.
- Durable state derives protocol readiness and relay subscription interest.
- Missing protocol state marks an account, direct chat, or group chat as not ready.
- Only already-signed relay publishes are retried.

This is intentionally not the final implementation plan. It is a subtractive spike whose output should be a failure map and a sharper follow-up plan.

## Phase 1: Subtractive Spike

Preserve signed-event relay publish retry. The existing `pending_relay_publishes` queue represents already-signed Nostr events, and retrying those events is delivery reliability rather than protocol repair.

Disable or remove protocol retry side effects:

- `queued_targets`
- pending outbound retries
- group fanout retries
- liveness-driven protocol retry

Disable or remove procedural protocol fetch/backfill triggered by missing state:

- `ProtocolEffect::FetchProtocolState`
- targeted queued fetches
- catch-up fetches whose only purpose is to unblock pending protocol work
- missing-state repair paths that repeatedly generate more protocol work

Keep subscription refresh from durable state. Login, foreground, reconnect, and protocol state changes should still recompute desired relay filters and apply subscriptions.

The key rule for this phase is:

```text
Missing protocol state never triggers retry side effects.
It only changes readiness and derived subscription interest.
```

## Phase 2: Failure Classification

After the subtractive spike compiles far enough to run tests, classify failures instead of restoring old behavior automatically.

Use these categories:

- Compile fallout from removed types, fields, or functions.
- Tests that encode old protocol backfill or retry behavior.
- Real product behavior that now needs explicit readiness state.
- Relay subscription recovery gaps.
- Publish retry regressions.

Publish retry regressions should be fixed immediately because signed-event relay retry stays in the simplified model. Other failures should feed the next design pass.

## Phase 3: Follow-Up Design Plan

Write the next implementation plan after the failure map is known.

The expected direction is:

- Add explicit readiness for account setup, direct chat initialization, and group chat initialization.
- Replace hidden protocol work loops with state-derived subscription interest.
- Reintroduce targeted fetch behavior only if tests prove subscriptions cannot provide required current state.
- Avoid an operation layer unless protocol side effects are intentionally regenerated from missing-state observations.

The intended end state is simpler than the current runtime: protocol inputs update state, state determines what the app is ready to do, and subscriptions determine what relay data the app is interested in.

## Assumptions

- This commit is docs-only.
- The spike is diagnostic, not a production-ready migration.
- Existing dev/test data may be cleared if needed during later implementation work.
- Publish retry remains in scope and must not be removed.
