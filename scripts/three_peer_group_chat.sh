#!/usr/bin/env bash

# Three-peer group chat interop, end-to-end through a local Rust relay.
# Verifies that an iOS, an Android, and a third Apple peer (iOS sim 2,
# which compiles from the same Swift sources as the macOS app, so the
# protocol-level guarantees apply transitively) all see each other's
# messages in a single group.
#
# Each peer:
#   * builds against the local relay set
#   * creates a fresh account
#   * is added to a group created by the iOS peer
#   * sends one message and waits for the others' messages to arrive
#
# No mocks, no harness shortcuts, no relayed-via-test-hooks. The harness
# is the same one the mixed-platform matrix uses; the difference is the
# device count (3 instead of 4) and the use of a second iOS simulator to
# stand in for the macOS app while the macOS XCTest harness is still WIP.
#
# Why a local relay (not iris.to): production relay propagation can drop
# the handshake into a >180s wait that's outside the test's control.
# The local relay removes propagation timing from the loop while still
# exercising the real ratchet, group, and transport code paths.

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"

LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "${SDK_DIR}" && -f "${LOCAL_PROPERTIES}" ]]; then
  SDK_DIR="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
fi
if [[ -z "${SDK_DIR}" ]]; then
  echo "Android SDK path not found." >&2
  exit 1
fi
export ANDROID_HOME="${SDK_DIR}"
ADB="${SDK_DIR}/platform-tools/adb"

ANDROID_HARNESS="${ROOT_DIR}/scripts/run_harness.py"
IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
ANDROID_RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS="to.iris.chat.RealRelayHarnessTest"
# Debug builds of the Android app live under `to.iris.chat.debug`
# (the gradle debug variant adds an applicationIdSuffix of ".debug").
# The test APK's instrumentation target also points at that suffixed
# id; clearing `to.iris.chat` would wipe an entirely different app
# the user might have installed from zapstore — and would leave the
# debug app's stale data in place across runs.
ANDROID_APP_PACKAGE="to.iris.chat.debug"
ANDROID_TEST_PACKAGE="to.iris.chat.test"

: "${ANDROID_SERIAL:?Set ANDROID_SERIAL to the authorized Android test device.}"
: "${IOS_PRIMARY_UDID:?Set IOS_PRIMARY_UDID to the primary iOS test simulator.}"
: "${IOS_MEMBER_UDID:?Set IOS_MEMBER_UDID to the secondary iOS test simulator.}"
IOS_PRIMARY_RUN_ID="${IOS_PRIMARY_RUN_ID:-three-peer-ios}"
IOS_MEMBER_RUN_ID="${IOS_MEMBER_RUN_ID:-three-peer-mac}"
GROUP_NAME="${GROUP_NAME:-ThreePeerGroup}"
RELAY_LOG="${RELAY_LOG:-/tmp/iris-three-peer-relay.log}"

extract_status() {
  awk -F= -v want="$1" '$0 ~ /^INSTRUMENTATION_STATUS: / {
    line=$0
    sub(/^INSTRUMENTATION_STATUS: /, "", line)
    eq=index(line, "=")
    if (eq>0) {
      key=substr(line, 1, eq-1)
      val=substr(line, eq+1)
      if (key == want) print val
    }
  }'
}

run_android_test() {
  local test_name="$1"
  shift
  local cmd=(
    python3 "${ANDROID_HARNESS}"
    --adb "${ADB}"
    --serial "${ANDROID_SERIAL}"
    --runner "${ANDROID_RUNNER}"
    --class-name "${ANDROID_CLASS}"
    --test-name "${test_name}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  printf '%s\n' "${output}"
  if ! printf '%s\n' "${output}" | grep -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "Android harness ${test_name} did not report success" >&2
    return 1
  fi
}

run_ios_test() {
  local udid="$1"
  local run_id="$2"
  local action="$3"
  shift 3
  local cmd=(
    python3 "${IOS_HARNESS}"
    --udid "${udid}"
    --run-id "${run_id}"
    --action "${action}"
  )
  if [[ "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--reset)
    if [[ "${run_id}" == "${IOS_PRIMARY_RUN_ID}" ]]; then
      cmd+=(--rebuild)
    fi
  fi
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  printf '%s\n' "${output}"
  if ! printf '%s\n' "${output}" | grep -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "iOS harness ${action} on ${run_id} did not report success" >&2
    return 1
  fi
}

require_value() {
  if [[ -z "${2:-}" ]]; then
    echo "Missing required value: $1" >&2
    return 1
  fi
}

cleanup() {
  local exit_code=$?
  if [[ -n "${RELAY_PID:-}" ]]; then
    stop_local_rust_relay "${RELAY_PID}"
  fi
  if [[ -n "${ANDROID_REVERSE_PORT:-}" ]]; then
    "${ADB}" -s "${ANDROID_SERIAL}" reverse --remove "tcp:${ANDROID_REVERSE_PORT}" >/dev/null 2>&1 || true
  fi
  if [[ ${exit_code} -ne 0 ]]; then
    echo "Three-peer group chat failed (exit ${exit_code})." >&2
    echo "Relay log: ${RELAY_LOG}" >&2
  fi
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" != "1" ]]; then
    iris_e2e_shutdown_ios_simulators "${IOS_PRIMARY_UDID}" "${IOS_MEMBER_UDID}"
  fi
  exit "${exit_code}"
}
trap cleanup EXIT

echo "==> Verifying devices are reachable"
"${ADB}" -s "${ANDROID_SERIAL}" get-state >/dev/null || {
  echo "Android device ${ANDROID_SERIAL} unreachable" >&2
  exit 1
}
iris_e2e_shutdown_stale_ios_simulators "${IOS_PRIMARY_UDID}" "${IOS_MEMBER_UDID}"
xcrun simctl list devices | grep -q "${IOS_PRIMARY_UDID}.*Booted" || xcrun simctl boot "${IOS_PRIMARY_UDID}"
xcrun simctl list devices | grep -q "${IOS_MEMBER_UDID}.*Booted" || xcrun simctl boot "${IOS_MEMBER_UDID}"
iris_e2e_wait_for_ios_bootstatus "${IOS_PRIMARY_UDID}"
iris_e2e_wait_for_ios_bootstatus "${IOS_MEMBER_UDID}"

echo "==> Starting local Rust relay"
RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
assert_local_relay_healthy
RELAY_PORT="$(local_relay_port)"
RELAY_SET_ID="$(local_relay_set_id)"
IOS_RELAY_URL="$(local_ios_relay_url)"

echo "==> Reverse-forwarding tcp:${RELAY_PORT} from Android device to host"
"${ADB}" -s "${ANDROID_SERIAL}" reverse "tcp:${RELAY_PORT}" "tcp:${RELAY_PORT}" >/dev/null
ANDROID_REVERSE_PORT="${RELAY_PORT}"
# Physical Android devices can't reach the host's loopback via 10.0.2.2,
# so we use adb reverse + 127.0.0.1 instead. Same scheme works for
# emulators if anyone ever swaps the device for an AVD.
ANDROID_RELAY_URL="ws://127.0.0.1:${RELAY_PORT}"

echo "==> Building Android app + test APK against ${ANDROID_RELAY_URL} (${RELAY_SET_ID})"
build_android_debug_apks "${ANDROID_RELAY_URL}" "${RELAY_SET_ID}"
install_android_debug_apks_on_serials "${ADB}" "${ANDROID_SERIAL}"

echo "==> Clearing Android app state"
"${ADB}" -s "${ANDROID_SERIAL}" shell pm clear "${ANDROID_APP_PACKAGE}" >/dev/null || true
"${ADB}" -s "${ANDROID_SERIAL}" shell pm clear "${ANDROID_TEST_PACKAGE}" >/dev/null || true

echo "==> Building iOS XCFramework against ${IOS_RELAY_URL} (${RELAY_SET_ID})"
(
  cd "${ROOT_DIR}" &&
    IRIS_DEFAULT_RELAYS="${IOS_RELAY_URL}" \
    IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
    IRIS_TRUSTED_TEST_BUILD=true \
    ./scripts/ios-build ios-xcframework
)

echo "==> Creating identities"
ANDROID_IDENTITY="$(run_android_test create_account_and_report_identity)"
ANDROID_NPUB="$(printf '%s\n' "${ANDROID_IDENTITY}" | extract_status npub)"
ANDROID_HEX="$(printf '%s\n' "${ANDROID_IDENTITY}" | extract_status public_key_hex)"
require_value android_npub "${ANDROID_NPUB}"
require_value android_hex "${ANDROID_HEX}"
echo "    Android peer: ${ANDROID_NPUB}"

IOS_PRIMARY_IDENTITY="$(run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_account_and_report_identity)"
IOS_PRIMARY_NPUB="$(printf '%s\n' "${IOS_PRIMARY_IDENTITY}" | extract_status npub)"
IOS_PRIMARY_HEX="$(printf '%s\n' "${IOS_PRIMARY_IDENTITY}" | extract_status public_key_hex)"
require_value ios_primary_npub "${IOS_PRIMARY_NPUB}"
require_value ios_primary_hex "${IOS_PRIMARY_HEX}"
echo "    iOS primary peer: ${IOS_PRIMARY_NPUB}"

IOS_MEMBER_IDENTITY="$(run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_account_and_report_identity)"
IOS_MEMBER_NPUB="$(printf '%s\n' "${IOS_MEMBER_IDENTITY}" | extract_status npub)"
IOS_MEMBER_HEX="$(printf '%s\n' "${IOS_MEMBER_IDENTITY}" | extract_status public_key_hex)"
require_value ios_member_npub "${IOS_MEMBER_NPUB}"
require_value ios_member_hex "${IOS_MEMBER_HEX}"
echo "    Apple/macOS-equivalent peer: ${IOS_MEMBER_NPUB}"

echo "==> Stabilising direct chat transports between all peers"
echo "    [stabilise 1/6] iOS primary -> Android create_chat"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_chat_from_args peer_input "${ANDROID_NPUB}" >/dev/null
echo "    [stabilise 2/6] iOS primary -> iOS member create_chat"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_chat_from_args peer_input "${IOS_MEMBER_NPUB}" >/dev/null
echo "    [stabilise 3/6] Android -> iOS primary create_chat"
run_android_test create_chat_from_args peer_input "${IOS_PRIMARY_NPUB}" >/dev/null
echo "    [stabilise 4/6] iOS member -> iOS primary create_chat"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_chat_from_args peer_input "${IOS_PRIMARY_NPUB}" >/dev/null
echo "    [stabilise 5/6] iOS primary wait_for_peer_transport_ready Android"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_peer_transport_ready_from_args peer_input "${ANDROID_NPUB}" >/dev/null
echo "    [stabilise 6/6] iOS primary wait_for_peer_transport_ready iOS member"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_peer_transport_ready_from_args peer_input "${IOS_MEMBER_NPUB}" >/dev/null

echo "==> Seeding direct messages so each peer's protocol state has a session"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" send_message_from_args peer_input "${ANDROID_NPUB}" message "seed_ios_to_android" >/dev/null
run_android_test wait_for_message_from_args peer_input "${IOS_PRIMARY_NPUB}" message "seed_ios_to_android" direction incoming >/dev/null
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" send_message_from_args peer_input "${IOS_MEMBER_NPUB}" message "seed_ios_to_mac" >/dev/null
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_message_from_args peer_input "${IOS_PRIMARY_NPUB}" message "seed_ios_to_mac" direction incoming >/dev/null

echo "==> iOS primary creates the group"
GROUP_CREATE="$(run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_group_from_args \
  group_name "${GROUP_NAME}" \
  member_inputs "${ANDROID_NPUB},${IOS_MEMBER_NPUB}")"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | extract_status chat_id)"
require_value group_chat_id "${GROUP_CHAT_ID}"
echo "    Group chat id: ${GROUP_CHAT_ID}"

echo "==> Android and Apple-equivalent peer wait for the group to land"
run_android_test wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null

echo "==> Each peer sends and the other two confirm receipt"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" send_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_ios" >/dev/null
run_android_test wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_ios" direction incoming >/dev/null
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_ios" direction incoming >/dev/null

run_android_test send_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_android" >/dev/null
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_android" direction incoming >/dev/null
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_android" direction incoming >/dev/null

run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" send_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_mac" >/dev/null
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_mac" direction incoming >/dev/null
run_android_test wait_for_message_from_args chat_id "${GROUP_CHAT_ID}" message "hello_from_mac" direction incoming >/dev/null

echo "==> All three peers exchanged messages successfully."
echo "relay_log=${RELAY_LOG}"
echo "android_serial=${ANDROID_SERIAL}"
echo "ios_primary_udid=${IOS_PRIMARY_UDID}"
echo "ios_member_udid=${IOS_MEMBER_UDID}"
echo "group_chat_id=${GROUP_CHAT_ID}"
