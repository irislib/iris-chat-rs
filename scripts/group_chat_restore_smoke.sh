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
PACKAGE_NAME="to.iris.chat.debug"
TEST_PACKAGE_NAME="to.iris.chat.test"
RELAY_LOG="${RELAY_LOG:-/tmp/ndr-group-restore-relay.log}"
RELAY_PID=""
DEFAULT_AVDS=("Medium_Phone_API_36.1" "Pixel_Fold")

PRIMARY_SERIAL="${PRIMARY_SERIAL:-}"
LINKED_SERIAL="${LINKED_SERIAL:-}"
ADMIN_SERIAL="${ADMIN_SERIAL:-}"
GROUP_NAME="${GROUP_NAME:-RestoreMatrixGroup}"
ADMIN_MESSAGE="${ADMIN_MESSAGE:-restore_matrix_admin_message}"
LINKED_MESSAGE="${LINKED_MESSAGE:-restore_matrix_linked_message}"
CLEAR_STATE=1

usage() {
  cat <<EOF
Usage: scripts/group_chat_restore_smoke.sh [options]

Options:
  --primary SERIAL      Primary-owner device serial. Default: auto-selected
  --linked SERIAL       Linked-device serial. Default: auto-selected
  --admin SERIAL        Admin/creator device serial. Default: auto-selected
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

add_unique_serial() {
  local serial="$1"
  local existing
  [[ -z "${serial}" ]] && return 0
  for existing in "${SELECTED_SERIALS[@]:-}"; do
    [[ "${existing}" == "${serial}" ]] && return 0
  done
  SELECTED_SERIALS+=("${serial}")
}

connected_serials() {
  "${ADB}" devices | awk 'NR>1 && $2 == "device" { print $1 }'
}

select_default_serials() {
  SELECTED_SERIALS=()
  add_unique_serial "${PRIMARY_SERIAL}"
  add_unique_serial "${LINKED_SERIAL}"
  add_unique_serial "${ADMIN_SERIAL}"

  if [[ -z "${PRIMARY_SERIAL}" || -z "${LINKED_SERIAL}" || -z "${ADMIN_SERIAL}" ]]; then
    local boot_output line serial
    boot_output="$("${ROOT_DIR}/scripts/run_android_emulators.sh" --headless "${DEFAULT_AVDS[@]}")"
    while IFS= read -r line; do
      serial="$(awk '{print $2}' <<<"${line}")"
      add_unique_serial "${serial}"
    done <<<"${boot_output}"

    while IFS= read -r serial; do
      add_unique_serial "${serial}"
    done < <(connected_serials)
  fi

  if [[ ${#SELECTED_SERIALS[@]} -lt 3 ]]; then
    echo "Need three Android devices for group restore smoke; found ${#SELECTED_SERIALS[@]}." >&2
    exit 1
  fi

  PRIMARY_SERIAL="${PRIMARY_SERIAL:-${SELECTED_SERIALS[0]}}"
  LINKED_SERIAL="${LINKED_SERIAL:-${SELECTED_SERIALS[1]}}"
  ADMIN_SERIAL="${ADMIN_SERIAL:-${SELECTED_SERIALS[2]}}"
}

select_default_serials

for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"; do
  if ! "${ADB}" -s "${serial}" get-state >/dev/null 2>&1; then
    echo "Device ${serial} is not online." >&2
    exit 1
  fi
done

cleanup() {
  if [[ -n "${RELAY_PID}" ]]; then
    stop_local_rust_relay "${RELAY_PID}"
  fi
}

if ! assert_local_relay_healthy >/dev/null 2>&1; then
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
fi

for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"; do
  "${ADB}" -s "${serial}" reverse "tcp:$(local_relay_port)" "tcp:$(local_relay_port)" >/dev/null || true
done

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

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
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

report_debug_snapshot() {
  local serial="$1"
  if [[ -n "${GROUP_CHAT_ID:-}" ]]; then
    echo "----- chat messages: ${serial} -----" >&2
    run_test "${serial}" report_chat_messages_from_args chat_id "${GROUP_CHAT_ID}" | tail -n 20 >&2 || true
  fi
  echo "----- debug snapshot: ${serial} -----" >&2
  run_test "${serial}" report_runtime_debug_snapshot | tail -n 80 >&2 || true
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
trap cleanup EXIT

if [[ "${CLEAR_STATE}" -eq 1 ]]; then
  for serial in "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"; do
    echo "Removing app state on ${serial}"
    "${ADB}" -s "${serial}" uninstall "${PACKAGE_NAME}" >/dev/null 2>&1 || true
    "${ADB}" -s "${serial}" uninstall "${TEST_PACKAGE_NAME}" >/dev/null 2>&1 || true
  done
fi

echo "Installing app and test APKs"
build_android_debug_apks "$(local_android_loopback_relay_url)" "$(local_relay_set_id)" >/dev/null
install_android_debug_apks_on_serials "${ADB}" "${PRIMARY_SERIAL}" "${LINKED_SERIAL}" "${ADMIN_SERIAL}"

echo "Creating primary owner on ${PRIMARY_SERIAL}"
PRIMARY_IDENTITY="$(run_test "${PRIMARY_SERIAL}" create_account_and_report_identity)"
PRIMARY_OWNER_NPUB="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status npub)"
PRIMARY_OWNER_HEX="$(printf '%s\n' "${PRIMARY_IDENTITY}" | extract_status public_key_hex)"

echo "Starting linked device on ${LINKED_SERIAL}"
LINK_LOG="$(mktemp -t iris-restore-link-log.XXXXXX)"
LINK_EXIT="$(mktemp -t iris-restore-link-exit.XXXXXX)"
IRIS_BACKGROUND_PID=""
run_test_background "${LINKED_SERIAL}" start_link_invite_and_wait_for_authorization_from_args \
  "${LINK_LOG}" "${LINK_EXIT}" \
  owner_input "${PRIMARY_OWNER_NPUB}" \
  authorization_state AUTHORIZED
LINK_PID="${IRIS_BACKGROUND_PID}"

LINK_URL="$(wait_for_status_in_file "${LINK_LOG}" invite_url 120)"

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
if [[ "${IRIS_RESTORE_SMOKE_TRACE_SEND:-0}" == "1" ]]; then
  LINKED_SEND_OUTPUT="$(run_test "${LINKED_SERIAL}" send_message_from_args \
    chat_id "${GROUP_CHAT_ID}" \
    message "${LINKED_MESSAGE}")"
  printf '%s\n' "${LINKED_SEND_OUTPUT}" >&2
  echo "----- post-send linked debug snapshot -----" >&2
  run_test "${LINKED_SERIAL}" report_runtime_debug_snapshot | tail -n 80 >&2 || true
else
  run_test "${LINKED_SERIAL}" send_message_from_args \
    chat_id "${GROUP_CHAT_ID}" \
    message "${LINKED_MESSAGE}" >/dev/null
fi
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
