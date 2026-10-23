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
    if [[ "$crate" == chat-protocol ]]; then
        # A large pending protocol backlog is independent of visible message
        # history. Keep this allocation/serialization/durability regression in
        # both CI and the tagged release gate, with its aggregate work counts.
        if ! checkpoint_output="$(cargo test "${args[@]}" --lib \
            protocol_engine::tests::checkpoint_backlog_production_counts_are_durable \
            -- --ignored --exact --test-threads=1 --nocapture 2>&1)"; then
            printf '%s\n' "$checkpoint_output"
            exit 1
        fi
        printf '%s\n' "$checkpoint_output"
        if [[ "$checkpoint_output" != *"test result: ok. 1 passed; 0 failed; 0 ignored;"* ||
              "$checkpoint_output" != *"checkpoint_backlog fanouts=22702 ciphertexts=19231 checkpoint_bytes=254672896"* ]]; then
            echo "Required protocol backlog checkpoint regression did not execute" >&2
            exit 1
        fi
        if [[ "$(uname -s)" == Linux ]]; then
            # One optimized protocol test, sharing the existing target/cache.
            # Retain complete stdout in the caller's log, including Cargo failure.
            # The subshell owns only its mktemp file; the outer relay trap remains.
            (
                batched_log="$(mktemp "${TMPDIR:-/tmp}/iris-batched-retry.XXXXXX")"
                trap 'rm -f "$batched_log"' EXIT
                CARGO_PROFILE_RELEASE_OPT_LEVEL=3 CARGO_PROFILE_RELEASE_DEBUG=1 \
                CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false \
                    cargo test "${args[@]}" --release --jobs 2 --lib \
                    protocol_engine::tests::restored_backlog_batched_retry_stays_within_cpu_budget \
                    -- --ignored --exact --test-threads=1 --nocapture 2>&1 | tee "$batched_log"
                python3 - "$batched_log" <<'BATCHED_CPU_RECEIPT'
import json
import math
from pathlib import Path
import re
import sys

text = Path(sys.argv[1]).read_text()
assert re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text) == [("ok", "1", "0", "0")], "Required batched retry test did not pass exactly once"
assert "running 1 test\n" in text
assert "protocol_engine::tests::restored_backlog_batched_retry_stays_within_cpu_budget" in text
rows = re.findall(r"batched_retry_cpu (\{[^\n]+\})", text)
assert len(rows) == 1, "Require exactly one complete batched retry receipt"
result = json.loads(rows[0])
expected = {
    "schema": "iris-batched-retry-cpu/v3", "fixture": "restored-mixed-backlog-with-corrupt-mac-inbound-v3", "synthetic": True,
    "inbound_ciphertext_format": "canonical-base64-nip44-v2-corrupt-mac",
    "inbound_ciphertext_format_checked": 4214,
    "pending_inbound": 4214, "initially_due_inbound": 102, "fanouts": 24990,
    "initially_due_fanouts": 10462, "ciphertexts": 23437, "remote_sends": 44,
    "expiring_controls": 2, "initial_remote_sends": 46, "retired_expiring_controls": 2,
        "checkpoint_bytes": 350224384, "outer_batch": True, "ack_and_flush_in_turn": True,
    "continuation_delay_ms": 25, "liveness_delay_ms": 2000, "settle_seconds": 180,
    "measurement_windows": 2, "window_seconds": 60, "debug_assertions": False,
    "initially_ready": True, "group_deliveries": 1, "acknowledged_deliveries": 2,
    "acknowledged_setup_deliveries": 1, "acknowledged_group_deliveries": 1,
    "repeated_search_starts": 0, "wire_effect_count": 0, "remaining_inbound": 4214,
    "remaining_ciphertexts": 23436, "remaining_remote_sends": 44,
    "preservation_checked": True, "restart_checked": True, "restart_ready": True,
}
assert all(type(result[key]) is type(value) and result[key] == value for key, value in expected.items()), "Batched workload or completion assertions changed"
assert all(type(result[key]) is int for key in ("inbound_decoded_min_bytes", "inbound_decoded_max_bytes"))
assert 387 <= result["inbound_decoded_min_bytes"] <= result["inbound_decoded_max_bytes"] <= 2115, "Inbound ciphertext sizes differ from the representative workload"
assert result["remaining_fanouts"] >= 24990 and result["search_starts"] > 0
assert all(type(result[key]) in (int, float) and math.isfinite(result[key]) and 0 <= result[key] <= 180
           for key in ("restore_seconds", "readiness_seconds", "first_valid_delivery_seconds")), "Recovery deadline not met"
slices = result["slices"]
emitted = [json.loads(row) for row in re.findall(r"batched_retry_slice (\{[^\n]+\})", text)]
assert len(slices) == 5 and emitted == slices, "Require three settling and two measured slices"
counts = ("search_starts", "repeated_search_starts", "input_generation_changes", "ready_candidates", "engine_attempts")
previous = None
elapsed_sum = 0.0
for index, row in enumerate(slices):
    assert (row["stage"], row["stage_index"]) == (("settle", index) if index < 3 else ("measure", index - 3)), "Slice order changed"
    for key in ("elapsed_seconds", "recovery_elapsed_seconds", "cpu_seconds", "cpu_percent_one_core", "work_seconds", "max_turn_seconds"):
        assert type(row[key]) in (int, float) and math.isfinite(row[key]) and row[key] >= 0, f"Invalid slice metric {key}"
    assert row["elapsed_seconds"] >= 60
    elapsed_sum += row["elapsed_seconds"]
    assert row["recovery_elapsed_seconds"] + 0.001 >= elapsed_sum
    assert math.isclose(row["cpu_percent_one_core"], 100 * row["cpu_seconds"] / row["elapsed_seconds"], rel_tol=1e-8, abs_tol=1e-8)
    phases = row["phase_work_seconds"]
    assert set(phases) == {"retirement", "group_inputs", "group_fanouts", "inbound", "remote_sends", "local_siblings", "flush", "decrypt_cursor_nested"}
    assert all(type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in phases.values())
    assert sum(value for name, value in phases.items() if name != "decrypt_cursor_nested") <= row["work_seconds"] + 0.01
    assert phases["decrypt_cursor_nested"] <= phases["group_inputs"] + 0.01
    for key in ("turns", "continuations", "acknowledged_deliveries", "puts", "read_bytes", "written_bytes", "remote_deadline_updates", "expired_control_retirements"):
        assert type(row[key]) is int and row[key] >= 0, f"Invalid slice counter {key}"
    assert all(type(row["due_at_start"][key]) is int and row["due_at_start"][key] >= 0 for key in ("inbound", "fanouts", "remote_sends"))
    before, after = row["progress_before"], row["progress_after"]
    for progress in (before, after):
        assert all(type(progress[key]) is int and progress[key] >= 0 for key in counts)
        assert all(type(progress[key]) is bool for key in ("active_cursor", "prepared_result"))
        assert progress["repeated_search_starts"] == 0, "Candidate re-admitted under unchanged inputs"
    assert all(after[key] >= before[key] for key in ("search_starts", "input_generation_changes"))
    assert previous is None or before == previous, "Audit reset between slices"
    previous = after
    assert row["expired_control_retirements"] == int(index >= 3), "Require real timed durable retirement in each measured minute"
    if index >= 3:
        assert row["turns"] > 0 and row["remote_deadline_updates"] > 0 and row["puts"] > 0, "Measured slice must include recurring due work and flush"
        assert row["cpu_percent_one_core"] <= 5, "Batched retry exceeded 5% of one CPU"
assert sum(row["acknowledged_deliveries"] for row in slices) == 2
assert all(result[key] == previous[key] for key in ("search_starts", "repeated_search_starts", "input_generation_changes"))
restart = result["restart_progress"]
assert all(type(restart[key]) is int and restart[key] >= 0 for key in counts)
assert all(type(restart[key]) is bool for key in ("active_cursor", "prepared_result"))
assert restart["search_starts"] == restart["repeated_search_starts"] == 0, "Restart readiness ran a key search"
print("Batched protocol CPU, progress and restart evidence verified")
BATCHED_CPU_RECEIPT
            )
        fi
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
