#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/idle-cpu-gate.py"
TMP="$(mktemp -d -t iris-chat-idle-cpu-test.XXXXXX)"
SLEEP_PID=""
BUSY_PID=""

cleanup() {
  if [[ -n "$SLEEP_PID" ]]; then
    kill "$SLEEP_PID" >/dev/null 2>&1 || true
    wait "$SLEEP_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$BUSY_PID" ]]; then
    kill "$BUSY_PID" >/dev/null 2>&1 || true
    wait "$BUSY_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

assert_status() {
  local expected="$1"
  shift
  local status=0
  "$@" >"$TMP/output.log" 2>&1 || status=$?
  if [[ "$status" -ne "$expected" ]]; then
    cat "$TMP/output.log" >&2
    echo "expected status $expected, got $status: $*" >&2
    exit 1
  fi
}

printf '%s\n' '{"loggedIn":true,"directChatCount":1,"groupChatCount":1}' >"$TMP/fixture.json"
sleep 10 &
SLEEP_PID=$!
assert_status 0 "$GATE" host-pid --pid "$SLEEP_PID" \
  --fixture "$TMP/fixture.json" --artifact "$TMP/idle.json" \
  --settle-seconds 0 --sample-seconds 0.2 --max-percent 5

python3 -c 'while True: pass' &
BUSY_PID=$!
assert_status 1 "$GATE" host-pid --pid "$BUSY_PID" \
  --fixture "$TMP/fixture.json" --artifact "$TMP/busy.json" \
  --settle-seconds 0 --sample-seconds 0.4 --max-percent 1

printf '%s\n' '{"loggedIn":true,"directChatCount":1,"groupChatCount":0}' >"$TMP/bad-fixture.json"
assert_status 1 "$GATE" host-pid --pid "$SLEEP_PID" \
  --fixture "$TMP/bad-fixture.json" --artifact "$TMP/bad.json" \
  --settle-seconds 0 --sample-seconds 0.1 --max-percent 5

python3 - "$TMP/idle.json" "$TMP/busy.json" "$TMP/bad.json" <<'PY'
import json
import sys

idle, busy, bad = (json.load(open(path, encoding="utf-8")) for path in sys.argv[1:])
assert idle["ok"] is True and idle["fixture"]["groupChatCount"] == 1
assert len(idle["processIds"]) == 1
assert busy["ok"] is False and busy["cpuPercent"] > busy["maxPercent"]
assert bad["ok"] is False and "no group chat" in bad["error"]
PY

for executable in \
  "$ROOT/scripts/idle-cpu-gate.py" \
  "$ROOT/scripts/idle-cpu-platform-gate.sh" \
  "$ROOT/scripts/linux-idle-cpu-gate-docker.sh" \
  "$ROOT/scripts/seed-idle-cpu-fixture.sh"; do
  [[ -x "$executable" ]] || { echo "idle CPU gate helper is not executable: $executable" >&2; exit 1; }
done

grep -q -- '--idle-cpu' "$ROOT/scripts/test-release-gate"
grep -q 'windows-idle-cpu' "$ROOT/scripts/windows-build"
grep -q 'create_chat_from_args' "$ROOT/scripts/idle-cpu-platform-gate.sh"
grep -q 'create_group_from_args' "$ROOT/scripts/idle-cpu-platform-gate.sh"
grep -q 'ios_simulator_is_booted' "$ROOT/scripts/idle-cpu-platform-gate.sh"
grep -q 'directChatCount' "$ROOT/scripts/idle-cpu-platform-gate-windows.ps1"
grep -q 'groupChatCount' "$ROOT/scripts/idle-cpu-platform-gate-windows.ps1"
grep -q 'IRIS_UI_TEST_RUN_ID' "$ROOT/windows/IrisChat/SingleInstanceService.cs"

echo "idle CPU gate harness passed"
