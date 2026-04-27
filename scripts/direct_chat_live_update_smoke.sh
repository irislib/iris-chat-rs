#!/usr/bin/env bash
# Reproduces the "messages appear only after navigating away and back" bug.
#
# Drives whichever two adb devices are *already* connected (physical
# device, running emulator, however you booted them) — no AVD auto-spawn.
# Override picks via DEVICE_A_SERIAL / DEVICE_B_SERIAL.
#
# A opens the chat with B, then B sends a message. A's harness asserts
# that the body lands in `state.currentChat.messages` *without* falling
# back to the chat-list preview / re-`OpenChat` workaround that masks
# the bug — that fallback fires `fetch_recent_protocol_state` which
# papers over exactly the regression we're chasing.
#
# Prereqs:
#   - Two devices visible to `adb devices` (physical + sim, two physical,
#     or two pre-booted emulators — your choice).
#   - The local Nostr relay running:  python3 scripts/local_nostr_relay.py
#   - Iris Chat debug + test APKs installed on each device.
#     `just android-assemble` and `cd android && ./gradlew :app:assembleDebugAndroidTest`
#     produce them; install with
#     `adb -s <serial> install -r app/build/outputs/apk/debug/app-debug.apk`
#     and likewise for `app-debug-androidTest.apk`.

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
HARNESS="${ROOT_DIR}/scripts/run_harness.py"
RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
TEST_CLASS="social.innode.ndr.demo.RealRelayHarnessTest"
TIMESTAMP="$(date +%s)"
MESSAGE="${MESSAGE:-live-update-${TIMESTAMP}}"

if [[ ! -x "${ADB}" ]]; then
  echo "adb not executable at ${ADB}" >&2
  exit 1
fi

assert_local_relay_healthy

connected_serials() {
  "${ADB}" devices | awk 'NR>1 && $2 == "device" { print $1 }'
}

mapfile -t SERIALS < <(connected_serials)

SERIAL_A="${DEVICE_A_SERIAL:-${SERIALS[0]:-}}"
SERIAL_B="${DEVICE_B_SERIAL:-${SERIALS[1]:-}}"

if [[ -z "${SERIAL_A}" || -z "${SERIAL_B}" || "${SERIAL_A}" == "${SERIAL_B}" ]]; then
  echo "Need two distinct adb devices online. Saw:" >&2
  printf '  - %s\n' "${SERIALS[@]:-<none>}" >&2
  echo "Connect a second device or boot a simulator/emulator, or set DEVICE_A_SERIAL / DEVICE_B_SERIAL explicitly." >&2
  exit 1
fi

echo "Device A: ${SERIAL_A}"
echo "Device B: ${SERIAL_B}"

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

echo "Provisioning identities on A and B"
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

# A is currently sitting on the chat with B. We do NOT call OpenChat
# again on A from here on — that's the whole point.
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
