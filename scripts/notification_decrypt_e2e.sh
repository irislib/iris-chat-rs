#!/usr/bin/env bash
# E2E proof that the FCM/APNs notification-decryption path actually
# works: a real DR session is established between two devices over the
# local Nostr relay, A sends Bob a DM, the kind:1060 wrapper that the
# notification server would forward is captured straight off the relay,
# and Bob's instrumentation feeds it to
# `AppManager.decryptOrResolveNotificationPayload` (the same call
# `IrisFirebaseMessagingService` makes in production). The test then
# asserts the resolved title is Alice's display name and the body is
# the plaintext message — not the generic "New activity" fallback.
#
# Like direct_chat_live_update_smoke.sh, this drives whichever two adb
# devices are already online; no AVD auto-spawn. Override with
# DEVICE_A_SERIAL / DEVICE_B_SERIAL.
#
# Prereqs:
#   - `python3 scripts/local_nostr_relay.py` running on the host and
#     reachable from both devices via `NDR_DEBUG_RELAYS=ws://<host>:4848`
#   - debug + test APKs installed on each device (`./gradlew :app:assembleDebug
#     :app:assembleDebugAndroidTest` then `adb install -r -t`).

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
RELAY_URL="${RELAY_URL:-ws://192.168.178.81:4848}"
RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
TEST_CLASS="social.innode.ndr.demo.RealRelayHarnessTest"
PACKAGE_NAME="to.iris.chat.debug"
TIMESTAMP="$(date +%s)"
ALICE_DISPLAY_NAME="${ALICE_DISPLAY_NAME:-Alice}"
MESSAGE="${MESSAGE:-decrypt-push-${TIMESTAMP}}"

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
  "${cmd[@]}"
}
extract_status() { sed -n "s/^INSTRUMENTATION_STATUS: $1=//p" | tail -n 1; }
require_value() { [[ -n "$2" ]] || { echo "missing $1" >&2; exit 1; }; }

echo "Wiping app state on both devices for a clean run"
"${ADB}" -s "${SERIAL_A}" shell pm clear "${PACKAGE_NAME}" >/dev/null || true
"${ADB}" -s "${SERIAL_B}" shell pm clear "${PACKAGE_NAME}" >/dev/null || true

echo "Provisioning identities and naming Alice"
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

# Bob's `core/profiles.json` needs to know Alice as ${ALICE_DISPLAY_NAME}
# so the decrypt path resolves the title to her display name. Alice
# publishes kind:0; Bob receives it via the same relay subscription that
# drives DR session events. We block on Bob seeing it before continuing.
echo "Seeding Alice's profile metadata as ${ALICE_DISPLAY_NAME}"
run_test "${SERIAL_A}" update_profile_metadata_from_args \
  display_name "${ALICE_DISPLAY_NAME}" >/dev/null

echo "Seeding session: A→B (drives DR bootstrap inline)"
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

echo "Force-stopping Bob's app to simulate the closed-app push case"
"${ADB}" -s "${SERIAL_B}" shell am force-stop "${PACKAGE_NAME}" >/dev/null

echo "Starting relay capture for B's incoming kind:1060"
CAPTURE_OUT="$(mktemp)"
python3 "${CAPTURE}" \
  --relay "${RELAY_URL}" \
  --kinds 1060 \
  --p-tag "${B_HEX}" \
  --since-secs 5 \
  --timeout-secs 60 \
  > "${CAPTURE_OUT}" &
CAPTURE_PID=$!
sleep 1

echo "Alice sends `${MESSAGE}` to Bob; capturing the wrapper off the relay"
run_test "${SERIAL_A}" send_message_from_args \
  peer_input "${B_NPUB}" \
  message "${MESSAGE}" >/dev/null

if ! wait "${CAPTURE_PID}"; then
  echo "Failed to capture outer event for Bob" >&2
  cat "${CAPTURE_OUT}" >&2
  exit 1
fi
OUTER_EVENT_JSON="$(cat "${CAPTURE_OUT}")"
rm -f "${CAPTURE_OUT}"
if [[ -z "${OUTER_EVENT_JSON}" ]]; then
  echo "Capture returned empty event" >&2
  exit 1
fi
echo "Captured ${#OUTER_EVENT_JSON} bytes of encrypted wrapper"

echo "Feeding wrapper to Bob's decryption path"
run_test "${SERIAL_B}" decrypt_notification_payload_from_args \
  outer_event_json "${OUTER_EVENT_JSON}" \
  expected_body "${MESSAGE}" \
  expected_title "${ALICE_DISPLAY_NAME}"

echo "Notification-decrypt e2e passed: Bob's notification renders <${ALICE_DISPLAY_NAME}>: ${MESSAGE}"
