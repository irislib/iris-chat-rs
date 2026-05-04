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
AM_USER="${AM_USER:-0}"

PRIMARY_SERIAL="${PRIMARY_SERIAL:-emulator-5554}"
LINKED_SERIAL="${LINKED_SERIAL:-emulator-5556}"
ADMIN_SERIAL="${ADMIN_SERIAL:-emulator-5558}"
MEMBER_SERIAL="${MEMBER_SERIAL:-5B011JEBF22130}"
GROUP_NAME="${GROUP_NAME:-MatrixGroup}"
SEED_PRIMARY_MESSAGE="${SEED_PRIMARY_MESSAGE:-matrix_seed_primary}"
SEED_MEMBER_MESSAGE="${SEED_MEMBER_MESSAGE:-matrix_seed_member}"
ADMIN_MESSAGE="${ADMIN_MESSAGE:-matrix_admin_message}"
LINKED_MESSAGE="${LINKED_MESSAGE:-matrix_linked_message}"
MEMBER_MESSAGE="${MEMBER_MESSAGE:-matrix_member_message}"
POST_REMOVE_MESSAGE="${POST_REMOVE_MESSAGE:-matrix_post_remove_message}"
REMOVED_MESSAGE="${REMOVED_MESSAGE:-matrix_removed_member_message}"
CLEAR_STATE=1

usage() {
  cat <<EOF
Usage: scripts/group_chat_matrix_smoke.sh [options]

Options:
  --primary SERIAL      Primary-owner device serial. Default: ${PRIMARY_SERIAL}
  --linked SERIAL       Linked-device serial. Default: ${LINKED_SERIAL}
  --admin SERIAL        Admin/creator device serial. Default: ${ADMIN_SERIAL}
  --member SERIAL       Independent member owner device serial. Default: ${MEMBER_SERIAL}
  --group-name NAME     Group name. Default: ${GROUP_NAME}
  --no-clear            Keep app state instead of clearing app packages first.
  -h, --help            Show this help.

Environment overrides:
  PRIMARY_SERIAL, LINKED_SERIAL, ADMIN_SERIAL, MEMBER_SERIAL, GROUP_NAME,
  SEED_PRIMARY_MESSAGE, SEED_MEMBER_MESSAGE, ADMIN_MESSAGE, LINKED_MESSAGE,
  MEMBER_MESSAGE, POST_REMOVE_MESSAGE, REMOVED_MESSAGE

What it validates:
  1. Primary owner account creation
  2. Linked device onboarding and authorization
  3. Admin owner account creation
  4. Independent member owner account creation
  5. Seed first-contact direct sessions from admin to primary and member
  6. Group create from admin to primary owner and member owner
  7. Group propagation to primary, linked, and member devices
  8. Group message send from admin, linked, and member
  9. Member removal from the group
  10. Removed member local send rejection
  11. Post-removal admin message delivery only to remaining members
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
    --member)
      MEMBER_SERIAL="$2"
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

for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}" "${MEMBER_SERIAL}"; do
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
  sleep 1

  local cmd=(
    python3
    "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${CLASS}"
    --test-name "${test_name}"
    --user "${AM_USER}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}"
    return 1
  }
  printf '%s\n' "${output}"
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_RESULT: shortMsg='; then
    echo "Instrumentation ${test_name} crashed on ${serial}" >&2
    return 1
  fi
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_FAILED:'; then
    echo "Instrumentation ${test_name} failed on ${serial}" >&2
    return 1
  fi
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "Instrumentation ${test_name} did not report success on ${serial}" >&2
    return 1
  fi
  sleep 1
}

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

# Run a harness test in the background, streaming its output into action_log.
# Sets the global IRIS_BACKGROUND_PID and writes the harness exit code into
# exit_file once the test finishes.
run_test_background() {
  local serial="$1"
  local test_name="$2"
  local action_log="$3"
  local exit_file="$4"
  shift 4

  "${ADB}" -s "${serial}" shell am force-stop "${TEST_PACKAGE_NAME}" >/dev/null 2>&1 || true

  local cmd=(
    python3
    "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${CLASS}"
    --test-name "${test_name}"
    --user "${AM_USER}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done

  : >"${action_log}"
  : >"${exit_file}"
  (
    set +e
    "${cmd[@]}" >"${action_log}" 2>&1
    printf '%s\n' "$?" >"${exit_file}"
  ) &
  IRIS_BACKGROUND_PID="$!"
}

wait_for_status_in_file() {
  local file="$1"
  local key="$2"
  local timeout_secs="$3"
  local deadline=$((SECONDS + timeout_secs))
  local value=""
  while (( SECONDS < deadline )); do
    if [[ -f "${file}" ]]; then
      value="$(extract_status "${key}" <"${file}")"
      if [[ -n "${value}" ]]; then
        printf '%s\n' "${value}"
        return 0
      fi
    fi
    sleep 1
  done
  echo "Timed out waiting for ${key} in ${file}" >&2
  return 1
}

require_value() {
  local name="$1"
  local value="$2"
  if [[ -z "${value}" ]]; then
    echo "Missing required status value: ${name}" >&2
    return 1
  fi
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
  echo "Matrix smoke failed with exit code ${exit_code}. Dumping device snapshots." >&2
  report_debug_snapshot "${PRIMARY_SERIAL}"
  report_debug_snapshot "${LINKED_SERIAL}"
  report_debug_snapshot "${ADMIN_SERIAL}"
  report_debug_snapshot "${MEMBER_SERIAL}"
  exit "${exit_code}"
}

trap dump_debug_on_error ERR

for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}" "${MEMBER_SERIAL}"; do
  if [[ "${serial}" != emulator-* ]]; then
    "${ADB}" -s "${serial}" shell svc power stayon usb >/dev/null 2>&1 || true
  fi
done

if [[ "${CLEAR_STATE}" -eq 1 ]]; then
  for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}" "${MEMBER_SERIAL}"; do
    echo "Clearing app state on ${serial}"
    "${ADB}" -s "${serial}" shell pm clear "${PACKAGE_NAME}" >/dev/null
    "${ADB}" -s "${serial}" shell pm clear "${TEST_PACKAGE_NAME}" >/dev/null || true
  done
fi

echo "Installing app and test APKs"
(cd "${ROOT_DIR}/android" && ./gradlew :app:installDebug :app:installDebugAndroidTest >/dev/null)

echo "Creating primary owner on ${PRIMARY_SERIAL}"
run_test "${PRIMARY_SERIAL}" create_account_and_report_identity >/dev/null
PRIMARY_IDENTITY="$(run_test "${PRIMARY_SERIAL}" report_logged_in_identity)"
PRIMARY_OWNER_NPUB="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status npub)"
PRIMARY_OWNER_HEX="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status public_key_hex)"
require_value PRIMARY_OWNER_NPUB "${PRIMARY_OWNER_NPUB}"
require_value PRIMARY_OWNER_HEX "${PRIMARY_OWNER_HEX}"

echo "Starting linked device on ${LINKED_SERIAL}"
LINK_LOG="$(mktemp -t iris-matrix-link-log.XXXXXX)"
LINK_EXIT="$(mktemp -t iris-matrix-link-exit.XXXXXX)"
IRIS_BACKGROUND_PID=""
run_test_background "${LINKED_SERIAL}" start_link_invite_and_wait_for_authorization_from_args \
  "${LINK_LOG}" "${LINK_EXIT}" \
  owner_input "${PRIMARY_OWNER_NPUB}" \
  authorization_state AUTHORIZED
LINK_PID="${IRIS_BACKGROUND_PID}"

LINK_URL="$(wait_for_status_in_file "${LINK_LOG}" invite_url 120)"
require_value LINK_URL "${LINK_URL}"

echo "Authorizing linked device on ${PRIMARY_SERIAL}"
run_test "${PRIMARY_SERIAL}" add_authorized_device_from_args \
  device_input "${LINK_URL}" >/dev/null

wait "${LINK_PID}" || true
LINK_STATUS="$(cat "${LINK_EXIT}")"
if [[ "${LINK_STATUS}" -ne 0 ]]; then
  echo "Linked-device authorization harness failed with exit code ${LINK_STATUS}" >&2
  cat "${LINK_LOG}" >&2 || true
  exit "${LINK_STATUS}"
fi
LINKED_IDENTITY="$(cat "${LINK_LOG}")"
LINKED_DEVICE_NPUB="$(printf '%s\n' "${LINKED_IDENTITY}" | extract_status device_npub)"
require_value LINKED_DEVICE_NPUB "${LINKED_DEVICE_NPUB}"
LINKED_AUTHORIZED_IDENTITY="$(run_test "${LINKED_SERIAL}" report_logged_in_identity)"
LINKED_OWNER_HEX="$(printf '%s\n' "${LINKED_AUTHORIZED_IDENTITY}" | extract_status public_key_hex)"
LINKED_OWNER_NPUB="$(printf '%s\n' "${LINKED_AUTHORIZED_IDENTITY}" | extract_status npub)"
require_value LINKED_OWNER_HEX "${LINKED_OWNER_HEX}"
require_value LINKED_OWNER_NPUB "${LINKED_OWNER_NPUB}"
if [[ "${LINKED_OWNER_HEX}" != "${PRIMARY_OWNER_HEX}" ]]; then
  echo "Linked device owner mismatch after authorization: expected ${PRIMARY_OWNER_HEX}, got ${LINKED_OWNER_HEX}" >&2
  exit 1
fi

echo "Creating admin owner on ${ADMIN_SERIAL}"
run_test "${ADMIN_SERIAL}" create_account_and_report_identity >/dev/null
ADMIN_IDENTITY="$(run_test "${ADMIN_SERIAL}" report_logged_in_identity)"
ADMIN_OWNER_NPUB="$(printf '%s\n' "${ADMIN_IDENTITY}" | extract_status npub)"
ADMIN_OWNER_HEX="$(printf '%s\n' "${ADMIN_IDENTITY}" | extract_status public_key_hex)"
require_value ADMIN_OWNER_NPUB "${ADMIN_OWNER_NPUB}"
require_value ADMIN_OWNER_HEX "${ADMIN_OWNER_HEX}"

echo "Creating independent member owner on ${MEMBER_SERIAL}"
run_test "${MEMBER_SERIAL}" create_account_and_report_identity >/dev/null
MEMBER_IDENTITY="$(run_test "${MEMBER_SERIAL}" report_logged_in_identity)"
MEMBER_OWNER_NPUB="$(printf '%s\n' "${MEMBER_IDENTITY}" | extract_status npub)"
MEMBER_OWNER_HEX="$(printf '%s\n' "${MEMBER_IDENTITY}" | extract_status public_key_hex)"
require_value MEMBER_OWNER_NPUB "${MEMBER_OWNER_NPUB}"
require_value MEMBER_OWNER_HEX "${MEMBER_OWNER_HEX}"

echo "Launching member app normally to keep its relay session alive"
"${ADB}" -s "${MEMBER_SERIAL}" shell am start --user "${AM_USER}" -n "${PACKAGE_NAME}/.MainActivity" >/dev/null
sleep 5

echo "Tracking primary and member owners on admin"
run_test "${ADMIN_SERIAL}" create_chat_from_args \
  peer_input "${PRIMARY_OWNER_NPUB}" >/dev/null
run_test "${ADMIN_SERIAL}" create_chat_from_args \
  peer_input "${MEMBER_OWNER_NPUB}" >/dev/null

echo "Bringing member app foreground and republishing local identity"
run_test "${MEMBER_SERIAL}" create_chat_from_args \
  peer_input "${ADMIN_OWNER_NPUB}" >/dev/null

echo "Waiting for admin to learn peer rosters and device invites"
run_test "${ADMIN_SERIAL}" wait_for_peer_transport_ready_from_args \
  peer_input "${PRIMARY_OWNER_NPUB}" >/dev/null
run_test "${ADMIN_SERIAL}" wait_for_peer_transport_ready_from_args \
  peer_input "${MEMBER_OWNER_NPUB}" >/dev/null

echo "Seeding direct session from admin to primary owner"
run_test "${ADMIN_SERIAL}" send_message_from_args \
  peer_input "${PRIMARY_OWNER_NPUB}" \
  message "${SEED_PRIMARY_MESSAGE}" >/dev/null
run_test "${PRIMARY_SERIAL}" wait_for_message_from_args \
  peer_input "${ADMIN_OWNER_NPUB}" \
  message "${SEED_PRIMARY_MESSAGE}" \
  direction incoming >/dev/null

echo "Seeding direct session from admin to independent member"
run_test "${ADMIN_SERIAL}" send_message_from_args \
  peer_input "${MEMBER_OWNER_NPUB}" \
  message "${SEED_MEMBER_MESSAGE}" >/dev/null

echo "Creating group on ${ADMIN_SERIAL}"
GROUP_CREATE="$(run_test "${ADMIN_SERIAL}" create_group_from_args \
  group_name "${GROUP_NAME}" \
  member_inputs "${PRIMARY_OWNER_NPUB},${MEMBER_OWNER_NPUB}")"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | extract_status chat_id)"
GROUP_ID="$(printf '%s\n' "${GROUP_CREATE}" | extract_status group_id)"
require_value GROUP_CHAT_ID "${GROUP_CHAT_ID}"
require_value GROUP_ID "${GROUP_ID}"

echo "Waiting for group on primary, linked, and member devices"
run_test "${PRIMARY_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_test "${LINKED_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_test "${MEMBER_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null

echo "Waiting for initial member count to converge"
for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}" "${MEMBER_SERIAL}"; do
  run_test "${serial}" wait_for_group_member_count_from_args \
    chat_id "${GROUP_CHAT_ID}" \
    member_count "3" >/dev/null
done

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
run_test "${MEMBER_SERIAL}" wait_for_message_from_args \
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
run_test "${MEMBER_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${LINKED_MESSAGE}" \
  direction incoming >/dev/null

echo "Sending group message from member"
run_test "${MEMBER_SERIAL}" send_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${MEMBER_MESSAGE}" >/dev/null
run_test "${ADMIN_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${MEMBER_MESSAGE}" \
  direction incoming >/dev/null
run_test "${PRIMARY_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${MEMBER_MESSAGE}" \
  direction incoming >/dev/null
run_test "${LINKED_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${MEMBER_MESSAGE}" \
  direction incoming >/dev/null

echo "Removing member ${MEMBER_OWNER_HEX} from group"
run_test "${ADMIN_SERIAL}" remove_group_member_from_args \
  group_id "${GROUP_ID}" \
  chat_id "${GROUP_CHAT_ID}" \
  member_input "${MEMBER_OWNER_HEX}" \
  expected_member_count "2" >/dev/null

echo "Waiting for post-removal member count to converge"
for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}" "${MEMBER_SERIAL}"; do
  run_test "${serial}" wait_for_group_member_count_from_args \
    chat_id "${GROUP_CHAT_ID}" \
    member_count "2" >/dev/null
done

echo "Verifying removed member local send rejection"
run_test "${MEMBER_SERIAL}" expect_send_rejected_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${REMOVED_MESSAGE}" >/dev/null

echo "Sending post-removal admin message"
run_test "${ADMIN_SERIAL}" send_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${POST_REMOVE_MESSAGE}" >/dev/null
run_test "${PRIMARY_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${POST_REMOVE_MESSAGE}" >/dev/null
run_test "${LINKED_SERIAL}" wait_for_message_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${POST_REMOVE_MESSAGE}" >/dev/null
run_test "${MEMBER_SERIAL}" assert_message_absent_from_args \
  chat_id "${GROUP_CHAT_ID}" \
  message "${POST_REMOVE_MESSAGE}" \
  timeout_ms "30000" >/dev/null

trap - ERR

echo "Group chat matrix smoke passed"
echo "primary=${PRIMARY_SERIAL}"
echo "linked=${LINKED_SERIAL}"
echo "admin=${ADMIN_SERIAL}"
echo "member=${MEMBER_SERIAL}"
echo "group_chat_id=${GROUP_CHAT_ID}"
echo "group_id=${GROUP_ID}"
echo "primary_owner_hex=${PRIMARY_OWNER_HEX}"
echo "admin_owner_hex=${ADMIN_OWNER_HEX}"
echo "member_owner_hex=${MEMBER_OWNER_HEX}"
