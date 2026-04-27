#!/usr/bin/env bash
# Reproduces the "messages appear only after navigating away and back" bug.
#
# Two devices, A and B, talk to each other through the local relay.
# A opens the chat with B, then B sends a message. A's harness asserts that
# the message lands in `state.currentChat.messages` *without* falling back
# to the chat-list preview / re-`OpenChat` workaround that masks the bug
# (the existing wait_for_message_from_args helper does exactly that).
#
# If this fails, the rerender regression is real: incoming DMs reach
# `threads` but the open-chat projection stays stale until OpenChat is
# dispatched again (which forces fetch_recent_protocol_state).

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "${SDK_DIR}" && -f "${LOCAL_PROPERTIES}" ]]; then
  SDK_DIR="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
fi
if [[ -z "${SDK_DIR}" ]]; then
  echo "Android SDK path not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or sdk.dir in local.properties." >&2
  exit 1
fi

ADB="${SDK_DIR}/platform-tools/adb"
EMULATOR="${SDK_DIR}/emulator/emulator"
HARNESS="${ROOT_DIR}/scripts/run_harness.py"
RUNNER="social.innode.irischat.test/androidx.test.runner.AndroidJUnitRunner"
TEST_CLASS="social.innode.ndr.demo.RealRelayHarnessTest"
DEFAULT_AVDS=("Pixel_9a" "Medium_Phone_API_36.1")
TIMESTAMP="$(date +%s)"
MESSAGE="${MESSAGE:-live-update-${TIMESTAMP}}"

if [[ ! -x "${ADB}" || ! -x "${EMULATOR}" ]]; then
  echo "adb / emulator not executable" >&2
  exit 1
fi

assert_local_relay_healthy

find_serial_for_avd() {
  local avd_name="$1"
  while read -r serial _; do
    [[ -z "${serial}" || "${serial}" == "List" ]] && continue
    local running_name
    running_name="$("${ADB}" -s "${serial}" emu avd name 2>/dev/null | head -n 1 | tr -d '\r')"
    if [[ "${running_name}" == "${avd_name}" ]]; then
      echo "${serial}"
      return 0
    fi
  done < <("${ADB}" devices | awk 'NR>1 && $2 == "device" { print $1, $2 }')
  return 1
}

ensure_avd_running() {
  local avd_name="$1"
  local serial
  serial="$(find_serial_for_avd "${avd_name}" || true)"
  if [[ -n "${serial}" ]]; then
    echo "${serial}"
    return 0
  fi
  local log_file="/tmp/${avd_name//[^A-Za-z0-9_.-]/_}.log"
  nohup "${EMULATOR}" -avd "${avd_name}" -no-window -no-audio -gpu swiftshader_indirect >"${log_file}" 2>&1 &
  for _ in {1..120}; do
    serial="$(find_serial_for_avd "${avd_name}" || true)"
    if [[ -n "${serial}" ]] &&
       "${ADB}" -s "${serial}" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' | grep -q '^1$'; then
      echo "${serial}"
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for ${avd_name} to boot." >&2
  exit 1
}

run_test() {
  local serial="$1"
  local test_name="$2"
  shift 2
  local cmd=(
    python3 "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${TEST_CLASS}"
    --test-name "${test_name}"
    --arg "clearPackageData=false"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  "${cmd[@]}"
}

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

require_value() {
  local label="$1"; local value="$2"
  if [[ -z "${value}" ]]; then
    echo "Missing required value ${label}" >&2
    exit 1
  fi
}

echo "Starting two emulators (${DEFAULT_AVDS[0]}, ${DEFAULT_AVDS[1]})"
SERIAL_A="$(ensure_avd_running "${DEFAULT_AVDS[0]}")"
SERIAL_B="$(ensure_avd_running "${DEFAULT_AVDS[1]}")"

echo "Provisioning identities on A=${SERIAL_A} and B=${SERIAL_B}"
A_OUT="$(run_test "${SERIAL_A}" create_account_and_report_identity)"
B_OUT="$(run_test "${SERIAL_B}" create_account_and_report_identity)"
A_NPUB="$(printf '%s\n' "${A_OUT}" | extract_status npub)"
B_NPUB="$(printf '%s\n' "${B_OUT}" | extract_status npub)"
require_value A_NPUB "${A_NPUB}"
require_value B_NPUB "${B_NPUB}"

echo "Establishing bi-directional DR session via a seed exchange"
run_test "${SERIAL_A}" create_chat_from_args peer_input "${B_NPUB}" >/dev/null
run_test "${SERIAL_B}" create_chat_from_args peer_input "${A_NPUB}" >/dev/null
run_test "${SERIAL_A}" wait_for_peer_transport_ready_from_args peer_input "${B_NPUB}" >/dev/null
run_test "${SERIAL_B}" wait_for_peer_transport_ready_from_args peer_input "${A_NPUB}" >/dev/null

run_test "${SERIAL_A}" send_message_from_args \
  peer_input "${B_NPUB}" \
  message "seed-from-A-${TIMESTAMP}" >/dev/null
run_test "${SERIAL_B}" wait_for_message_from_args \
  peer_input "${A_NPUB}" \
  message "seed-from-A-${TIMESTAMP}" \
  direction incoming >/dev/null

# A is currently sitting on the chat with B (last create_chat_from_args
# call put it there). We do NOT call OpenChat again on A — that's the
# whole point.
echo "B sends a fresh message; A must surface it in the open chat"
run_test "${SERIAL_B}" send_message_from_args \
  peer_input "${A_NPUB}" \
  message "${MESSAGE}" >/dev/null

# Strict variant — no chatList fallback, no implicit OpenChat. If the
# message never reaches state.currentChat.messages this fails with a
# message that distinguishes "didn't arrive at all" from
# "arrived in chatList but the open-chat projection stayed stale".
run_test "${SERIAL_A}" wait_for_incoming_message_in_open_chat_strict_from_args \
  peer_input "${B_NPUB}" \
  message "${MESSAGE}" \
  timeout_ms 60000

echo "Live-update smoke passed: A surfaced B's message without re-navigation"
