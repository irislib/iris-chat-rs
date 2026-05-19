#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${IRIS_SENDER_KEY_FAULT_MATRIX_RUN_DIR:-/tmp/iris-sender-key-fault-matrix-${RUN_ID}}"
MODE="all"
EXACT=()
NOCAPTURE=0

usage() {
  cat <<EOF
Usage: scripts/sender_key_fault_matrix.sh [--basic|--adversarial] [--exact NAME] [--nocapture]

Runs the deterministic sender-key fault matrix.

Options:
  --basic          Run only the 10 basic scenarios.
  --adversarial   Run only the 10 adversarial scenarios.
  --exact NAME    Run exactly one named scenario. Repeatable.
  --nocapture     Pass --nocapture to Rust tests.
  -h, --help      Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --basic)
      MODE="basic"
      shift
      ;;
    --adversarial)
      MODE="adversarial"
      shift
      ;;
    --exact)
      EXACT+=("$2")
      shift 2
      ;;
    --nocapture)
      NOCAPTURE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

BASIC_SCENARIOS=(
  local_relay_drop_file_exact_once
  local_relay_drop_file_always
  pending_publish_inspector_pairwise
  pending_publish_inspector_group_outer
  sender_key_create_three_members
  sender_key_send_all_members
  sender_key_restart_all_members
  sender_key_multi_device_owner
  sender_key_add_late_member
  sender_key_remove_member
)

ADVERSARIAL_SCENARIOS=(
  sender_key_outer_before_distribution
  sender_key_outer_before_metadata
  sender_key_drop_distribution_then_repair
  sender_key_drop_metadata_then_repair
  sender_key_repair_survives_receiver_restart
  sender_key_repair_survives_sender_restart
  sender_key_duplicate_replay_idempotent
  sender_key_removed_member_repair_denied
  sender_key_mixed_order_storm
  sender_key_cli_drop_repair_flow
)

if ((${#EXACT[@]} > 0)); then
  SCENARIOS=("${EXACT[@]}")
elif [[ "${MODE}" == "basic" ]]; then
  SCENARIOS=("${BASIC_SCENARIOS[@]}")
elif [[ "${MODE}" == "adversarial" ]]; then
  SCENARIOS=("${ADVERSARIAL_SCENARIOS[@]}")
else
  SCENARIOS=("${BASIC_SCENARIOS[@]}" "${ADVERSARIAL_SCENARIOS[@]}")
fi

mkdir -p "${RUN_DIR}"
echo "sender_key_fault_matrix_run_dir=${RUN_DIR}"

run_logged() {
  local scenario="$1"
  shift
  local log="${RUN_DIR}/${scenario}.log"
  echo "=== ${scenario} ==="
  printf 'command:' >"${log}"
  printf ' %q' "$@" >>"${log}"
  printf '\n' >>"${log}"
  "$@" 2>&1 | tee -a "${log}"
}

run_core_lib_test() {
  local scenario="$1"
  local filter="$2"
  local tail_args=(--test-threads=1)
  if [[ "${NOCAPTURE}" == "1" ]]; then
    tail_args+=(--nocapture)
  fi
  run_logged "${scenario}" cargo test --manifest-path "${ROOT_DIR}/core/Cargo.toml" --lib "${filter}" -- "${tail_args[@]}"
}

run_cli_test() {
  local scenario="$1"
  local filter="$2"
  local tail_args=(--test-threads=1)
  if [[ "${NOCAPTURE}" == "1" ]]; then
    tail_args+=(--nocapture)
  fi
  run_logged "${scenario}" cargo test --manifest-path "${ROOT_DIR}/core/Cargo.toml" --test cli_interop "${filter}" -- "${tail_args[@]}"
}

run_relay_drop_smoke() {
  local scenario="$1"
  local always="$2"
  run_logged "${scenario}" python3 - "${ROOT_DIR}" "${always}" <<'PY'
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

root = pathlib.Path(sys.argv[1])
always = sys.argv[2] == "always"
sys.path.insert(0, str(root / "scripts"))
from capture_relay_event import encode_frame, open_websocket, read_frame

with tempfile.TemporaryDirectory() as d:
    drop = pathlib.Path(d) / "drop.txt"
    drop.write_text("drop-me\n")
    env = os.environ.copy()
    env["IRIS_LOCAL_RELAY_DROP_EVENT_IDS_FILE"] = str(drop)
    if always:
        env["IRIS_LOCAL_RELAY_DROP_EVENT_IDS_ALWAYS"] = "1"
    proc = subprocess.Popen(
        [str(root / "scripts/local_nostr_relay.py"), "127.0.0.1:4861"],
        cwd=root,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        deadline = time.time() + 90
        sock = None
        while time.time() < deadline:
            if proc.poll() is not None:
                raise AssertionError(proc.stdout.read())
            try:
                sock = open_websocket("ws://127.0.0.1:4861", timeout=1)
                break
            except Exception:
                time.sleep(0.25)
        assert sock is not None, "relay did not become ready"
        sock.settimeout(3)
        event = {"id":"drop-me","pubkey":"author","created_at":1,"kind":1060,"tags":[], "content":"abc"}
        sock.sendall(encode_frame(json.dumps(["EVENT", event]).encode()))
        if not always:
            sock.sendall(encode_frame(json.dumps(["EVENT", event]).encode()))
        ok_count = 0
        while ok_count < (1 if always else 2):
            opcode, payload = read_frame(sock)
            if opcode == 1:
                msg = json.loads(payload.decode())
                if msg and msg[0] == "OK":
                    ok_count += 1
        sock.sendall(encode_frame(json.dumps(["REQ", "sub", {"ids":["drop-me"]}]).encode()))
        seen = []
        while True:
            opcode, payload = read_frame(sock)
            if opcode != 1:
                continue
            msg = json.loads(payload.decode())
            if msg[0] == "EVENT":
                seen.append(msg[2]["id"])
            if msg[0] == "EOSE":
                break
        if always:
            assert seen == [], seen
        else:
            assert seen == ["drop-me"], seen
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
PY
}

run_pending_inspector_smoke() {
  local scenario="$1"
  local mode="$2"
  run_logged "${scenario}" python3 - "${ROOT_DIR}" "${mode}" <<'PY'
import base64
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

root = pathlib.Path(sys.argv[1])
mode = sys.argv[2]
with tempfile.TemporaryDirectory() as d:
    db = pathlib.Path(d) / "core.sqlite3"
    conn = sqlite3.connect(db)
    conn.execute("""create table pending_relay_publishes (
        event_id text primary key,
        owner_pubkey_hex text not null,
        label text not null,
        event_json text not null,
        inner_event_id text,
        target_owner_pubkey_hex text,
        target_device_id text,
        message_id text,
        chat_id text,
        created_at_secs integer not null,
        attempt_count integer not null default 0,
        last_error text
    )""")
    pairwise = {"id":"pairwise-id","pubkey":"author1","created_at":1,"kind":1060,"tags":[["header","abc"]], "content":"cipher"}
    outer_content = base64.b64encode(b"\x00\x00\x00\x01\x00\x00\x00\x00cipher").decode()
    outer = {"id":"outer-id","pubkey":"author2","created_at":2,"kind":1060,"tags":[], "content":outer_content}
    conn.execute("insert into pending_relay_publishes values (?,?,?,?,?,?,?,?,?,?,?,?)", ("pairwise-id","owner","pairwise",json.dumps(pairwise),None,"bob-owner","bob-device",None,None,1,0,None))
    conn.execute("insert into pending_relay_publishes values (?,?,?,?,?,?,?,?,?,?,?,?)", ("outer-id","owner","outer",json.dumps(outer),None,None,None,None,None,2,0,None))
    conn.commit()
    conn.close()
    if mode == "pairwise":
        args = ["list", "--data-dir", d, "--target-owner-hex", "bob-owner", "--pairwise-only", "--format", "ids"]
        expected = "pairwise-id"
    else:
        args = ["list", "--data-dir", d, "--group-sender-outer-only", "--format", "ids"]
        expected = "outer-id"
    out = subprocess.check_output([str(root / "scripts/pending_relay_publishes.py"), *args], text=True).strip()
    assert out == expected, out
PY
}

run_scenario() {
  local scenario="$1"
  case "${scenario}" in
    local_relay_drop_file_exact_once)
      run_relay_drop_smoke "${scenario}" once
      ;;
    local_relay_drop_file_always)
      run_relay_drop_smoke "${scenario}" always
      ;;
    pending_publish_inspector_pairwise)
      run_pending_inspector_smoke "${scenario}" pairwise
      ;;
    pending_publish_inspector_group_outer)
      run_pending_inspector_smoke "${scenario}" group_outer
      ;;
    sender_key_create_three_members)
      run_core_lib_test "${scenario}" appcore_sender_key_group_create_prepares_pairwise_metadata_and_distribution
      ;;
    sender_key_send_all_members)
      run_core_lib_test "${scenario}" appcore_sender_key_four_member_matrix_delivers_one_outer_per_sender
      ;;
    sender_key_restart_all_members)
      run_core_lib_test "${scenario}" appcore_sender_key_pending_outer_survives_restart_and_applies_once
      ;;
    sender_key_multi_device_owner)
      run_cli_test "${scenario}" sender_key_cli_group_interop_three_members_restart_and_restored_owner_device
      ;;
    sender_key_add_late_member)
      run_core_lib_test "${scenario}" appcore_sender_key_late_member_and_remove_member_enforce_membership_window
      ;;
    sender_key_remove_member)
      run_core_lib_test "${scenario}" appcore_sender_key_late_member_and_remove_member_enforce_membership_window
      ;;
    sender_key_outer_before_distribution)
      run_core_lib_test "${scenario}" appcore_sender_key_outer_before_distribution_retries_after_control_state
      ;;
    sender_key_outer_before_metadata)
      run_core_lib_test "${scenario}" appcore_sender_key_missing_metadata_revision_repairs_and_applies_pending_outer
      ;;
    sender_key_drop_distribution_then_repair)
      run_core_lib_test "${scenario}" appcore_sender_key_missing_rotated_distribution_repairs_and_applies_pending_outer
      ;;
    sender_key_drop_metadata_then_repair)
      run_core_lib_test "${scenario}" appcore_sender_key_missing_metadata_revision_repairs_and_applies_pending_outer
      ;;
    sender_key_repair_survives_receiver_restart)
      run_core_lib_test "${scenario}" appcore_sender_key_repair_request_survives_restart_and_throttles
      ;;
    sender_key_repair_survives_sender_restart)
      run_core_lib_test "${scenario}" appcore_sender_key_repair_response_survives_sender_restart
      ;;
    sender_key_duplicate_replay_idempotent)
      run_core_lib_test "${scenario}" appcore_sender_key_duplicate_replay_idempotent
      ;;
    sender_key_removed_member_repair_denied)
      run_core_lib_test "${scenario}" appcore_sender_key_removed_member_repair_denied
      ;;
    sender_key_mixed_order_storm)
      run_core_lib_test "${scenario}" appcore_sender_key_mixed_order_storm_converges
      ;;
    sender_key_cli_drop_repair_flow)
      run_cli_test "${scenario}" sender_key_cli_group_interop_three_members_restart_and_restored_owner_device
      ;;
    *)
      echo "Unknown scenario: ${scenario}" >&2
      echo "Known scenarios:" >&2
      printf '  %s\n' "${BASIC_SCENARIOS[@]}" "${ADVERSARIAL_SCENARIOS[@]}" >&2
      return 1
      ;;
  esac
}

for scenario in "${SCENARIOS[@]}"; do
  run_scenario "${scenario}"
done

echo "Sender-key fault matrix passed: ${#SCENARIOS[@]} scenario(s)"
echo "Artifacts: ${RUN_DIR}"
