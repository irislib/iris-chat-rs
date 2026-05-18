#!/usr/bin/env bash
# E2E proof that a received group message can wake the mobile push path,
# locally decrypt the captured relay event, and render the notification as:
#   "<sender name> in <group name>": "<message preview>"
#
# Android lane:
#   - drives two already-online adb devices
#   - creates Alice and Bob
#   - Alice creates a group with Bob
#   - captures Alice's group sender-key event from the relay using Bob's
#     mobile push subscription authors
#   - feeds that event into Bob's Android notification decrypt path

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "${SDK_DIR}" && -f "${LOCAL_PROPERTIES}" ]]; then
  SDK_DIR="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
fi
if [[ -z "${SDK_DIR}" ]]; then
  echo "Android SDK path not found." >&2
  exit 1
fi

ADB="${SDK_DIR}/platform-tools/adb"
HARNESS="${ROOT_DIR}/scripts/run_harness.py"
CAPTURE="${ROOT_DIR}/scripts/capture_relay_event.py"
RELAY_URL="${RELAY_URL:-$(local_android_loopback_relay_url)}"
CAPTURE_SINCE_SECS="${CAPTURE_SINCE_SECS:-120}"
RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
TEST_CLASS="to.iris.chat.RealRelayHarnessTest"
PACKAGE_NAME="${PACKAGE_NAME:-to.iris.chat.debug}"
TIMESTAMP="$(date +%s)"
ALICE_DISPLAY_NAME="${ALICE_DISPLAY_NAME:-Alice}"
GROUP_NAME="${GROUP_NAME:-Push Test Group}"
MESSAGE="${MESSAGE:-group-decrypt-push-${TIMESTAMP}}"
EXPECTED_TITLE="${EXPECTED_TITLE:-${GROUP_NAME}}"
EXPECTED_BODY="${EXPECTED_BODY:-${ALICE_DISPLAY_NAME}: ${MESSAGE}}"

usage() {
  cat <<EOF
Usage: scripts/notification_decrypt_group_e2e.sh

Drives two already-online Android adb devices through a real relay-backed
group message notification decrypt test.

Environment overrides:
  DEVICE_A_SERIAL, DEVICE_B_SERIAL, RELAY_URL, ALICE_DISPLAY_NAME, GROUP_NAME,
  MESSAGE, EXPECTED_TITLE, PACKAGE_NAME, CAPTURE_SINCE_SECS
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

assert_local_relay_healthy

connected_serials() {
  "${ADB}" devices | awk 'NR>1 && $2 == "device" { print $1 }'
}

SERIALS=()
while IFS= read -r line; do [[ -n "${line}" ]] && SERIALS+=("${line}"); done < <(connected_serials)
SERIAL_A="${DEVICE_A_SERIAL:-${SERIALS[0]:-}}"
SERIAL_B="${DEVICE_B_SERIAL:-${SERIALS[1]:-}}"
if [[ -z "${SERIAL_A}" || -z "${SERIAL_B}" || "${SERIAL_A}" == "${SERIAL_B}" ]]; then
  echo "Need two distinct adb devices online." >&2
  printf '  - %s\n' "${SERIALS[@]:-<none>}" >&2
  exit 1
fi
echo "Device A (sender):   ${SERIAL_A}"
echo "Device B (receiver): ${SERIAL_B}"

run_test() {
  local serial="$1"; local name="$2"; shift 2
  local cmd=(
    python3 "${HARNESS}"
    --adb "${ADB}" --serial "${serial}"
    --runner "${RUNNER}" --class-name "${TEST_CLASS}" --test-name "${name}"
    --arg "clearPackageData=false"
  )
  while [[ $# -gt 0 ]]; do cmd+=(--arg "$1=$2"); shift 2; done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}"
    return 1
  }
  printf '%s\n' "${output}"
  if printf '%s\n' "${output}" | grep -Eq '^(FAILURES!!!|INSTRUMENTATION_STATUS_CODE: -[0-9]|Error in |INSTRUMENTATION_FAILED:)'; then
    return 1
  fi
}

extract_status() { sed -n "s/^INSTRUMENTATION_STATUS: $1=//p" | tail -n 1; }
require_value() { [[ -n "$2" ]] || { echo "missing $1" >&2; exit 1; }; }

echo "Wiping app state on both devices for a clean run"
"${ADB}" -s "${SERIAL_A}" shell pm clear "${PACKAGE_NAME}" >/dev/null || true
"${ADB}" -s "${SERIAL_B}" shell pm clear "${PACKAGE_NAME}" >/dev/null || true

echo "Provisioning identities"
A_OUT="$(run_test "${SERIAL_A}" create_account_and_report_identity)"
A_NPUB="$(printf '%s\n' "${A_OUT}" | extract_status npub)"
A_HEX="$(printf '%s\n' "${A_OUT}" | extract_status public_key_hex)"
require_value A_NPUB "${A_NPUB}"
require_value A_HEX "${A_HEX}"
B_OUT="$(run_test "${SERIAL_B}" create_account_and_report_identity)"
B_NPUB="$(printf '%s\n' "${B_OUT}" | extract_status npub)"
B_HEX="$(printf '%s\n' "${B_OUT}" | extract_status public_key_hex)"
require_value B_NPUB "${B_NPUB}"
require_value B_HEX "${B_HEX}"

echo "Seeding Alice's profile metadata as ${ALICE_DISPLAY_NAME}"
run_test "${SERIAL_A}" update_profile_metadata_from_args \
  display_name "${ALICE_DISPLAY_NAME}" >/dev/null

echo "Seeding direct session so Bob learns Alice's profile"
run_test "${SERIAL_A}" send_message_from_args \
  peer_input "${B_NPUB}" \
  message "seed-${TIMESTAMP}" >/dev/null
run_test "${SERIAL_B}" wait_for_message_from_args \
  chat_id "${A_HEX}" \
  message "seed-${TIMESTAMP}" \
  direction incoming >/dev/null
run_test "${SERIAL_B}" wait_for_peer_profile_name_from_args \
  peer_pubkey_hex "${A_HEX}" \
  display_name "${ALICE_DISPLAY_NAME}" >/dev/null

echo "Creating group '${GROUP_NAME}' with Bob"
GROUP_OUT="$(run_test "${SERIAL_A}" create_group_from_args \
  group_name "${GROUP_NAME}" \
  member_inputs "${B_NPUB}")"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_OUT}" | extract_status chat_id)"
GROUP_ID="$(printf '%s\n' "${GROUP_OUT}" | extract_status group_id)"
require_value group_chat_id "${GROUP_CHAT_ID}"
require_value group_id "${GROUP_ID}"

echo "Waiting for Bob to receive the group"
run_test "${SERIAL_B}" wait_for_group_chat_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  timeout_secs 180 >/dev/null

echo "Reading Bob's active mobile push authors"
B_PUSH_OUT="$(run_test "${SERIAL_B}" report_mobile_push_snapshot)"
B_MESSAGE_AUTHORS="$(printf '%s\n' "${B_PUSH_OUT}" | extract_status message_author_pubkeys)"
require_value message_author_pubkeys "${B_MESSAGE_AUTHORS}"
if ! printf '%s' "${B_MESSAGE_AUTHORS}" | grep -Eq '(^|,)[0-9a-f]{64}(,|$)'; then
  echo "Bob's push author snapshot did not contain any event authors: ${B_MESSAGE_AUTHORS}" >&2
  exit 1
fi

echo "Force-stopping Bob's app to simulate the closed-app push case"
"${ADB}" -s "${SERIAL_B}" shell am force-stop "${PACKAGE_NAME}" >/dev/null

echo "Alice sends '${MESSAGE}' to ${GROUP_NAME}"
SEND_OUT="$(run_test "${SERIAL_A}" send_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${MESSAGE}")"
OUTER_EVENT_IDS="$(printf '%s\n' "${SEND_OUT}" | extract_status outer_event_ids)"
OUTER_EVENT_ID="${OUTER_EVENT_IDS%%,*}"
require_value outer_event_id "${OUTER_EVENT_ID}"

echo "Capturing exact group wrapper ${OUTER_EVENT_ID} off the relay"
CAPTURE_OUT="$(mktemp)"
if ! python3 "${CAPTURE}" \
  --relay "${RELAY_URL}" \
  --kinds 1060 \
  --id "${OUTER_EVENT_ID}" \
  --since-secs "${CAPTURE_SINCE_SECS}" \
  --timeout-secs 30 \
  > "${CAPTURE_OUT}"; then
  echo "Failed to capture group outer event ${OUTER_EVENT_ID}" >&2
  cat "${CAPTURE_OUT}" >&2
  exit 1
fi
OUTER_EVENT_JSON="$(cat "${CAPTURE_OUT}")"
rm -f "${CAPTURE_OUT}"
if [[ -z "${OUTER_EVENT_JSON}" ]]; then
  echo "Capture returned empty event" >&2
  exit 1
fi
echo "Captured ${#OUTER_EVENT_JSON} bytes of encrypted group wrapper"

echo "Feeding wrapper to Bob's decryption path"
run_test "${SERIAL_B}" decrypt_notification_payload_from_args \
  outer_event_json "${OUTER_EVENT_JSON}" \
  expected_body "${EXPECTED_BODY}" \
  expected_title "${EXPECTED_TITLE}"

echo "Group notification-decrypt e2e passed: Bob's notification renders <${EXPECTED_TITLE}>: ${EXPECTED_BODY}"
