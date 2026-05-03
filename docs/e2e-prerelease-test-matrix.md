# Pre-Release E2E Test Matrix

This document tracks the long confidence suite for the experimental NDR core/runtime work. These flows are intentionally opt-in and are not part of normal CI. The goal is to exercise the same risks through fast runtime-only tests and slower mobile E2E setups before a release.

Default public relay set:

```text
wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to
```

## Setups

| Setup | Entry Point | Default Relay Mode | Purpose |
| --- | --- | --- | --- |
| Runtime-only | `cargo test -p nostr-double-ratchet prerelease` and `cargo test prerelease` in `core` | In-memory/fake runtime | Fast protocol and app-state coverage without mobile devices. |
| iOS public | `scripts/e2e_ios_public_prerelease.sh --fresh` | Public | Four iOS simulators: Alice primary, Alice linked, Bob, Charlie. |
| Android public | `IRIS_ANDROID_E2E_AVDS="A B C D" scripts/e2e_android_public_prerelease.sh --fresh` | Public | Four Android emulators with the same topology. |
| Mixed public | `IRIS_ANDROID_E2E_AVDS="A B" scripts/e2e_mixed_public_prerelease.sh --fresh` | Public | Alice is multi-device across iOS and Android. |
| Local recovery | `scripts/e2e_prerelease_matrix.sh --relay local --flow <id>` | Local | Deterministic relay stop/start and recovery cases. |

Each script writes a timestamped trace under `/tmp/iris-e2e-<setup>-<stamp>/` with repo SHAs, relay configuration, build path, device IDs, identities, link URL, group IDs, harness output, and final debug snapshots.

## Coverage Matrix

Status values:

- `Covered`: implemented as an automated or scripted flow.
- `Partial`: important pieces are covered, but the exact end-to-end case still needs a longer script or harness parity.
- `Planned`: documented target for the pre-release suite.

| ID | Flow | Runtime-only | iOS | Android | Mixed | Public Relay | Local Relay | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| F01 | Fresh four-device baseline: Alice primary, Alice linked, Bob, Charlie; link device, seed direct chats, verify self-sync, create group, rename group. | Partial | Covered | Covered | Partial | Covered | Planned | Main public confidence flow. Runtime tests cover the protocol pieces separately. |
| F02 | Cross-platform linked user: Alice primary on one platform, Alice linked on the other; Bob and Charlie split across platforms. | Partial | N/A | N/A | Covered | Covered | Planned | Mixed script defaults to Alice primary iOS and Alice linked Android. |
| F03 | Restart after link authorization while AppKeys/link bootstrap are settling. | Partial | Planned | Planned | Planned | Partial | Covered | Needs controllable restart timing in scripts. |
| F04 | Restart after prepared direct send before publish confirmation; verify replay without ratchet advancement or duplicate message. | Covered | Planned | Planned | Planned | Partial | Covered | Runtime already exercises prepared publish durability. |
| F05 | Restart after group creation before all members observe the group; verify control fanout completes. | Covered | Planned | Planned | Planned | Partial | Covered | Runtime exercises queued group fanout. |
| F06 | Offline relay recovery: queue direct and group sends while relay is unavailable, restart apps, restore relay, verify delivery and no duplicates. | Partial | Planned | Planned | Planned | N/A | Covered | Must use local relay mode. |
| F07 | Delayed AppKeys/invite gaps: missing roster or invite queues outbound work and retries after protocol events arrive. | Covered | Partial | Partial | Planned | Partial | Covered | Runtime tests cover deterministic gap handling. |
| F08 | Group add/remove members: add a member, deliver snapshot/sender key, remove a member, verify rejected send from removed member. | Partial | Planned | Covered | Planned | Planned | Covered | Android smoke already covers removal and rejection. |
| F09 | Group admin changes: promote/demote admin, verify allowed mutations propagate and non-admin mutations are rejected. | Partial | Planned | Planned | Planned | Planned | Covered | Requires harness actions for admin assertions. |
| F10 | Linked-device revocation: revoke Alice linked device, verify revoked state and rejected sends. | Partial | Planned | Covered | Planned | Planned | Covered | Android linked-device matrix already covers revocation. |
| F11 | Delayed sender-key distribution: group outer arrives before pairwise distribution, then decrypts after distribution. | Covered | Planned | Planned | Planned | N/A | Covered | Runtime test covers pending sender-key outer replay. |
| F12 | App-level parity: typing, delivered/seen receipts, reactions, chat settings, disappearing messages, restart around expiry. | Partial | Planned | Planned | Planned | Planned | Covered | Core tests cover pieces; mobile harness parity is incomplete. |
| F13 | Backfill/self-sync edge: linked device offline during early same-owner sends, later catches up without duplicate routing. | Partial | Planned | Planned | Planned | Covered | Covered | Public relays are useful here because backfill behavior matters. |
| F14 | Multi-restart soak across direct sends, group sends, membership changes, and relay delays. | Planned | Planned | Planned | Planned | Planned | Covered | Long pre-release soak, not a smoke test. |

## Baseline Acceptance

The baseline mobile scripts must pass these checks before manual testing continues:

- All selected devices boot and run a freshly built app for the configured relay set.
- Alice primary, Bob, and Charlie create clean accounts in `--fresh` mode.
- Alice linked device is authorized and reports Alice's owner identity.
- Bob and Charlie receive Alice's direct seed messages.
- Alice linked receives at least one same-owner sender copy.
- Alice creates a group with Bob and Charlie.
- Bob, Charlie, and Alice linked all observe `group:<group_id>`.
- Alice renames the group and each recipient observes the renamed group.
- Final runtime and persisted protocol snapshots are written for every device.

## Harness Gaps

The pre-release scripts use the existing harnesses where possible. Remaining long-flow work should prioritize:

- iOS app-level event assertions for receipts, reactions, chat settings, and disappearing-message expiry.
- Android app-level event assertions for receipts, reactions, chat settings, and disappearing-message expiry.
- Negative admin assertions on both mobile harnesses, for example non-admin mutation rejection.
- Runtime-only tests for full app-level flows that currently live above the NDR runtime, especially reactions, receipts, and disappearing-message expiry.
