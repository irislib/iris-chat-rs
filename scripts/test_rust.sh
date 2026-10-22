#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${ROOT_DIR}/core"
# Reuse dependencies across the separate manifests without sharing build locks
# with other worktrees. Explicit caller overrides still take precedence.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/core/target}"

# Device approval has its own build-time relay, separate from SetNostrRelays.
# Keep FFI integration tests local unless the caller supplies a fixture.
if [[ -z "${IRIS_DEVICE_APPROVAL_RELAY_URL:-}" ]]; then
    approval_fixture="$(mktemp -d "${TMPDIR:-/tmp}/iris-rust-approval.XXXXXX")"
    cleanup_approval_fixture() {
        python3 "${ROOT_DIR}/scripts/persistent_local_nostr_relay.py" stop \
            --pid-file "${approval_fixture}/relay.pid" >/dev/null 2>&1 || true
        rm -rf "${approval_fixture}"
    }
    trap cleanup_approval_fixture EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    approval_port="$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
)"
    python3 "${ROOT_DIR}/scripts/persistent_local_nostr_relay.py" start \
        --bind "127.0.0.1:${approval_port}" --host 127.0.0.1 --port "${approval_port}" \
        --pid-file "${approval_fixture}/relay.pid" --log-file "${approval_fixture}/relay.log"
    export IRIS_DEVICE_APPROVAL_RELAY_URL="ws://127.0.0.1:${approval_port}"
fi

# The published ICE archive now carries the consent/candidate fixes. Its exact
# checksum is pinned by test_dependency_lock_consistency.py; the package's own
# release checks cover those regressions. Do not compile the inactive old vendor.

# Preserve the capped-process calibration and peak-memory evidence in Linux
# release logs; successful output is otherwise hidden by either suite runner.
if [[ "$(uname -s)" == "Linux" ]]; then
    cargo test --manifest-path "${ROOT_DIR}/core/Cargo.toml" --locked --lib \
        core::protocol::anonymous_reads::memory_tests::populated_account_discovery_under_memory_limit \
        -- --exact --nocapture
fi

# Prefer cargo-nextest when available: it runs test binaries in parallel
# (cargo test runs them serially), which makes a big difference for the
# CLI integration tests. Fall back to cargo test if nextest isn't installed.
for crate in core chat-protocol protocol-ffi; do
    args=(--manifest-path "${ROOT_DIR}/${crate}/Cargo.toml" --locked)
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run --no-fail-fast "${args[@]}"
        cargo test -q --doc "${args[@]}"
    else
        cargo test -q "${args[@]}"
    fi
    # The populated CPU test runs once in the ordinary nonignored core suite.
    # Nextest captures successful stdout, so CI retains its companion files with
    # the existing native private-control artifact instead of rerunning it.
    if [[ "$crate" == core && "$(uname -s)" == Linux && -n "${IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT:-}" ]]; then
        python3 - "$IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT" <<'CPU_RECEIPT'
import hashlib
import json
import math
from pathlib import Path
import sys

root = Path(sys.argv[1]).parent
receipt = json.loads((root / "populated-discovery-idle.json").read_text())
log = (root / "populated-discovery-idle.log").read_bytes()
assert receipt["schema"] == "iris-populated-discovery-idle/v1"
assert receipt["passed"] is True and receipt["timed_out"] is False and receipt["exit_code"] == 0
assert receipt["log_bytes"] == len(log) and receipt["log_sha256"] == hashlib.sha256(log).hexdigest()
assert len(receipt["measurements"]) == 1
result = receipt["measurements"][0]
assert result["database_bytes"] >= 100 * 1024 * 1024 and result["message_rows"] >= 20800
assert (result["contacts"], result["peer_devices"], result["own_devices"], result["groups"]) == (96, 288, 3, 8)
assert result["discovery_subjects"] == 384 and result["readiness_round_min"] >= 3
assert result["mesh_discovery_subjects"] == 96
assert result["total_discovery_subjects"] == result["discovery_subjects"] + result["mesh_discovery_subjects"] == 480
assert result["completed_peer_polls"] >= result["total_discovery_subjects"] * result["readiness_round_min"]
assert result["fresh_anonymous_authentications"] >= result["completed_peer_polls"] + 1
assert result["revoked_stale_records"] >= 96
assert result["targeted_fresh_proof_delivered"] is True and result["privacy_wire_passed"] is True
samples = result["quiet_samples"]
assert len(samples) == 2 and sum(sample["seconds"] for sample in samples) >= 70
for sample in samples:
    assert math.isfinite(sample["cpu_percent_one_core"]) and 0 <= sample["cpu_percent_one_core"] <= 5
    assert sample["application_wire_bytes"] == sample["connections"] == 0
cycles = result["full_cycle_tls_samples"]
assert len(cycles) == 2
for cycle in cycles:
    assert math.isfinite(cycle["seconds"]) and cycle["seconds"] >= 125
    assert math.isfinite(cycle["cpu_percent_one_core"]) and 0 <= cycle["cpu_percent_one_core"] <= 5
    assert cycle["every_author_polled"] is True
    assert cycle["tls_full_handshakes"] == 0
    assert cycle["tls_resumed_handshakes"] >= result["total_discovery_subjects"]
    assert cycle["connections"] >= result["total_discovery_subjects"] and cycle["application_wire_bytes"] > 0
print("Populated discovery CPU/wire companion evidence verified")
CPU_RECEIPT
    fi
done
