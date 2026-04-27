#!/usr/bin/env bash

set -Eeuo pipefail

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
if [[ ! -x "${ADB}" ]]; then
  echo "adb not found at ${ADB}" >&2
  exit 1
fi

if [[ ! -f "${HARNESS}" ]]; then
  echo "Harness runner not found at ${HARNESS}" >&2
  exit 1
fi

RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
CLASS="to.iris.chat.RealRelayHarnessTest"
PACKAGE_NAME="to.iris.chat"
TEST_PACKAGE_NAME="to.iris.chat.test"

PRIMARY_SERIAL="${PRIMARY_SERIAL:-emulator-5554}"
LINKED_SERIAL="${LINKED_SERIAL:-emulator-5556}"
ADMIN_SERIAL="${ADMIN_SERIAL:-emulator-5558}"
GROUP_NAME="${GROUP_NAME:-RestoreMatrixGroup}"
ADMIN_MESSAGE="${ADMIN_MESSAGE:-restore_matrix_admin_message}"
LINKED_MESSAGE="${LINKED_MESSAGE:-restore_matrix_linked_message}"
CLEAR_STATE=1

usage() {
  cat <<EOF
Usage: scripts/group_chat_restore_smoke.sh [options]

Options:
  --primary SERIAL      Primary-owner device serial. Default: ${PRIMARY_SERIAL}
  --linked SERIAL       Linked-device serial. Default: ${LINKED_SERIAL}
  --admin SERIAL        Admin/creator device serial. Default: ${ADMIN_SERIAL}
  --group-name NAME     Group name. Default: ${GROUP_NAME}
  --no-clear            Keep app state instead of clearing both app packages first.
  -h, --help            Show this help.

Environment overrides:
  PRIMARY_SERIAL, LINKED_SERIAL, ADMIN_SERIAL, GROUP_NAME, ADMIN_MESSAGE, LINKED_MESSAGE

What it validates:
  1. Primary owner account creation
  2. Linked device onboarding and authorization
  3. Admin owner account creation
  4. Group create from admin to the primary owner
  5. Admin app force-stop and restore
  6. Group propagation to primary and linked devices
  7. Group message send from admin to both devices
  8. Group message send from linked device to admin, with sibling copy on primary
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --primary)
      PRIMARY_SERIAL="$2"
      shift 2
      ;;
    --linked)
      LINKED_SERIAL="$2"
      shift 2
      ;;
    --admin)
      ADMIN_SERIAL="$2"
      shift 2
      ;;
    --group-name)
      GROUP_NAME="$2"
      shift 2
      ;;
    --no-clear)
      CLEAR_STATE=0
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

for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"; do
  if ! "${ADB}" -s "${serial}" get-state >/dev/null 2>&1; then
    echo "Device ${serial} is not online." >&2
    exit 1
  fi
done

assert_local_relay_healthy

run_test() {
  local serial="$1"
  local test_name="$2"
  shift 2

  "${ADB}" -s "${serial}" shell am force-stop "${TEST_PACKAGE_NAME}" >/dev/null 2>&1 || true

  local cmd=(
    python3
    "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${CLASS}"
    --test-name "${test_name}"
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

report_debug_snapshot() {
  local serial="$1"
  echo "----- debug snapshot: ${serial} -----" >&2
  run_test "${serial}" report_runtime_debug_snapshot | tail -n 30 >&2 || true
  echo "----- persisted snapshot: ${serial} -----" >&2
  run_test "${serial}" report_persisted_protocol_snapshot | tail -n 20 >&2 || true
}

dump_debug_on_error() {
  local exit_code=$?
  echo "Smoke script failed with exit code ${exit_code}. Dumping device snapshots." >&2
  report_debug_snapshot "${PRIMARY_SERIAL}"
  report_debug_snapshot "${LINKED_SERIAL}"
  report_debug_snapshot "${ADMIN_SERIAL}"
  exit "${exit_code}"
}

trap dump_debug_on_error ERR

if [[ "${CLEAR_STATE}" -eq 1 ]]; then
  for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"; do
    echo "Clearing app state on ${serial}"
    "${ADB}" -s "${serial}" shell pm clear "${PACKAGE_NAME}" >/dev/null
    "${ADB}" -s "${serial}" shell pm clear "${TEST_PACKAGE_NAME}" >/dev/null || true
  done
fi

echo "Installing app and test APKs"
(cd "${ROOT_DIR}/android" && ./gradlew :app:installDebug :app:installDebugAndroidTest >/dev/null)

echo "Creating primary owner on ${PRIMARY_SERIAL}"
PRIMARY_IDENTITY="$(run_test "${PRIMARY_SERIAL}" create_account_and_report_identity)"
PRIMARY_OWNER_NPUB="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status npub)"
PRIMARY_OWNER_HEX="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status public_key_hex)"

echo "Starting linked device on ${LINKED_SERIAL}"
LINKED_IDENTITY="$(run_test "${LINKED_SERIAL}" start_linked_device_and_report_identity \
  owner_input "${PRIMARY_OWNER_NPUB}")"
LINKED_DEVICE_NPUB="$(printf '%s\n' "${LINKED_IDENTITY}" | extract_status device_npub)"

echo "Authorizing linked device on ${PRIMARY_SERIAL}"
run_test "${PRIMARY_SERIAL}" add_authorized_device_from_args \
  device_input "${LINKED_DEVICE_NPUB}" >/dev/null
run_test "${LINKED_SERIAL}" wait_for_authorization_state_from_args \
  authorization_state AUTHORIZED >/dev/null

echo "Creating admin owner on ${ADMIN_SERIAL}"
ADMIN_IDENTITY="$(run_test "${ADMIN_SERIAL}" create_account_and_report_identity)"
ADMIN_OWNER_HEX="$(printf '%s\n' "${ADMIN_IDENTITY}" | extract_status public_key_hex)"

echo "Creating group on ${ADMIN_SERIAL}"
GROUP_CREATE="$(run_test "${ADMIN_SERIAL}" create_group_from_args \
  group_name "${GROUP_NAME}" \
  member_inputs "${PRIMARY_OWNER_NPUB}")"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | extract_status chat_id)"

echo "Force-stopping admin app to exercise restore"
"${ADB}" -s "${ADMIN_SERIAL}" shell am force-stop "${PACKAGE_NAME}"
"${ADB}" -s "${ADMIN_SERIAL}" shell monkey -p "${PACKAGE_NAME}" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
sleep 4
run_test "${ADMIN_SERIAL}" report_runtime_debug_snapshot >/dev/null

echo "Waiting for group on primary and linked devices"
run_test "${PRIMARY_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_test "${LINKED_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null

echo "Sending group message from admin"
run_test "${ADMIN_SERIAL}" send_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${ADMIN_MESSAGE}" >/dev/null
run_test "${PRIMARY_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${ADMIN_MESSAGE}" >/dev/null
run_test "${LINKED_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${ADMIN_MESSAGE}" >/dev/null

echo "Sending group message from linked device"
run_test "${LINKED_SERIAL}" send_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${LINKED_MESSAGE}" >/dev/null
run_test "${ADMIN_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${LINKED_MESSAGE}" \
  direction incoming >/dev/null
run_test "${PRIMARY_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${LINKED_MESSAGE}" \
  direction outgoing >/dev/null

trap - ERR

echo "Group chat restore smoke passed"
echo "primary=${PRIMARY_SERIAL}"
echo "linked=${LINKED_SERIAL}"
echo "admin=${ADMIN_SERIAL}"
echo "group_chat_id=${GROUP_CHAT_ID}"
echo "primary_owner_hex=${PRIMARY_OWNER_HEX}"
echo "admin_owner_hex=${ADMIN_OWNER_HEX}"
