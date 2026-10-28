# Testing Fixtures

Use the core fixture builders when a test needs lots of chats, groups, or
messages but does not need relay/protocol behavior:

- `build_large_test_app_state(direct_chat_count, group_chat_count, messages_in_current_chat)`
- `build_large_test_search_result(query, person_count, contact_count, group_count, message_count)`

These are deterministic and capped so UI/perf tests can safely ask for large
datasets. They are exposed through UniFFI for shell tests and also usable from
Rust integration tests.

Shell contract tests should default to these builders for logged-in app states
and search mocks. Hand-build a tiny `AppState` only when the behavior under test
depends on an exact empty/login/reset state or on a narrow field transition that
would be hidden by fixture data.

Use the local relay for protocol and persistence behavior. The in-process Rust
fixture is `iris_chat_core::local_relay::TestRelay`; the standalone binary is:

```sh
cargo run --manifest-path core/Cargo.toml --features local-relay-bin --bin local_nostr_relay -- 127.0.0.1:4848
```

Use production relays only for smoke tests that must prove interoperability with
the live network. Keep performance/rendering tests deterministic so failures are
not caused by relay availability or account history.

## Populated native VM profile

For an explicitly authorized full-app development measurement, the opt-in
exporter generates a dedicated account with at least 100 MiB of history, 50
contacts with three devices each, three groups, and 137 cache-only owners.
It uses real SQLite/search indexes, ratchet checkpoints, attachment storage and
Swift account restoration. It never copies an existing account or uses a mock core.

Arrange the shared resource lease/budget first; the focused runner does not
acquire one. Run from the repository root with a compatible existing
`CARGO_TARGET_DIR`. Create new absolute output paths and a distinct run ID:

```sh
python3 - <<'PYINPUT'
from pathlib import Path
import json, os
os.umask(0o077)
base = Path("work").resolve()
base.mkdir(mode=0o700, exist_ok=True)
inputs = {
    "output": str(base / "synthetic-vm-profile-01"),
    "run_id": "synthetic-vm-local-01",
    "relay_urls": [
        "wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net",
        "wss://relay.snort.social", "wss://temp.iris.to"
    ]
}
with (base / "synthetic-vm-inputs.json").open("x") as stream:
    json.dump(inputs, stream, indent=2)
PYINPUT

IRIS_SYNTHETIC_VM_INPUTS="$PWD/work/synthetic-vm-inputs.json" \
python3 scripts/test-rust-focused --crate core --jobs 2 --timeout 300 \
  --output "$PWD/work/synthetic-vm-checks-01" --include-ignored \
  --test core::tests::synthetic_vm_profile::synthetic_vm_export_rejects_unsafe_or_ambiguous_inputs \
  --test core::tests::synthetic_vm_profile::synthetic_vm_seed_corpus_rejects_unsafe_unbounded_or_expired_events \
  --test core::tests::synthetic_vm_profile::synthetic_vm_export_reopens_real_history_and_swift_account_bundle \
  --test core::tests::synthetic_history_small_fixture_exercises_real_database_and_ffi \
  --test core::tests::synthetic_vm_profile::export_populated_native_vm_profile
```

Select five distinct TLS relays appropriate for the test. The runner compiles
once, runs four checks, then executes the explicitly selected ignored export.
Keep `profile/` private: its file secret store and databases contain generated
account/session keys. Preserve the complete tree, including attachments and empty
directories, and verify every receipt hash after transfer (0700 directories,
0600 files). Keep the exact source, input, app binary and receipts together.

Seed only an authorized test target in a separate step. Only `relay-events.json`
is eligible: at most 290 signed AppKeys/device-invite records, under 1 MiB, with
signed expiration within 24 hours. No messages, shared group records or private
state belong in that publication. Require acknowledgments and readback before
calling the discovery workload populated; empty replies are not equivalent.
Record which configured relays were seeded and read back; do not imply all five
hold the fixture when only one was seeded.

Launch the unchanged app with the receipt's `IRIS_UI_TEST_RUN_ID`,
`IRIS_UI_TEST_DATA_DIR` set to the transferred `profile/`,
`IRIS_UI_TEST_BYPASS_KEYCHAIN=1`. Record the exact launch environment. The
export receipt lists `IRIS_DISABLE_NOTIFICATIONS=1` as an optional setting;
omit it to retain ordinary notification behavior. Do not set `IRIS_UI_TEST_RESET`.
Storage is isolated; production TLS, FIPS and WebRTC remain in use, with
production nearby defaults.
Leave other apps/profiles intact. After the first networked launch, upgrade this
same profile in place. Never run an active clone or roll back its ratchet state;
generate a new account when a fresh fixture is needed.

Record guest resources, foreground readiness, intervals and relay availability.
This synthetic live-network VM workload is development evidence, not a direct
comparison with a user's account or tagged-release acceptance. `RELEASE.md`
still governs release artifacts and physical-device checks.
