# Verification tiers and native lab

Iris Chat separates deterministic per-change confidence from native GUI and
physical-device confidence.

## Fast tier

Run `just verify-fast` for each coherent change. It runs the native-lab unit
tests, Rust formatting, strict all-target Clippy, static contracts, and Rust
tests. Android compilation, iOS tests, simulators, phones, VMs, and GUI sessions
are intentionally deferred.

## Full tier

Run `just verify-full` nightly and before release candidates. It runs the fast
tier, reserves the native matrix, then runs all platform tests and the full
interop/on-device release gate. The reliability simulator/emulator lane is on
by default; set `IRIS_VERIFY_FULL_RELIABILITY=0` only for a deliberately smaller
release investigation. Set `IRIS_VERIFY_FULL_MACOS_VM=1` to add the macOS VM
public-relay prerelease journey.

The full release gate includes idle CPU checks for the macOS, iOS, Android,
Linux, and Windows app shells when their configured runners are available. Each
lane uses an isolated logged-in account with at least one direct chat and one
group chat, settles for 30 seconds, samples for 60 seconds, and blocks above 5%
of one core. Results are stored under `artifacts/idle-cpu/`. Set
`IRIS_TEST_GATE_IDLE_CPU=0` only when intentionally excluding this release
criterion.

Configure `IRIS_WINDOWS_SSH_HOST` explicitly. Local allocations use:

- `IRIS_CHAT_LAB_IOS_SIMULATOR`
- `IRIS_CHAT_LAB_IOS_DEVICE`
- `IRIS_CHAT_LAB_ANDROID_SERIAL`

Names, UDIDs, and serials are accepted; `auto` is the default for local mobile
resources. Prefer explicit values in scheduled jobs.

`scripts/native_lab.py` atomically reserves the matrix plus the local Mac,
Windows host, simulator, and phones. It passes the exact selected IDs into the
test process and writes `artifacts/verification/full-native-result.json`. Missing
or busy infrastructure exits 75 with status `infrastructure_unavailable`.
Product failures retain their command exit and status `product_failure`.

Use `just verify-health` to preflight without running tests.

## Deterministic resets

Reset is destructive and off by default. Use
`IRIS_NATIVE_LAB_RESET=1 just verify-full` only with dedicated lab targets. The
wrapper erases the reserved iOS simulator and clears the Iris Chat app/test
packages on the reserved Android target before the matrix starts.

`scripts/native_state_reset.sh` requires
`IRIS_NATIVE_LAB_ALLOW_RESET=1`; the full wrapper sets this only while holding
the reservation. Do not select a personal phone if its Iris Chat data should be
preserved.
