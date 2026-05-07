#!/usr/bin/env bash

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
RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
PACKAGE_NAME="to.iris.chat.debug"
TEST_PACKAGE_NAME="to.iris.chat.test"
DEFAULT_AVDS=("Medium_Phone_API_36.1" "Pixel_Fold")
RELAY_LOG="${RELAY_LOG:-/tmp/ndr-linked-device-relay.log}"
RELAY_PID=""
SERIAL_A="${SERIAL_A:-}"
SERIAL_B="${SERIAL_B:-}"
SERIAL_C="${SERIAL_C:-}"

if [[ ! -x "${ADB}" ]]; then
  echo "adb not found at ${ADB}" >&2
  exit 1
fi

if [[ ! -x "${EMULATOR}" ]]; then
  echo "emulator not found at ${EMULATOR}" >&2
  exit 1
fi

if [[ ! -f "${HARNESS}" ]]; then
  echo "Harness runner not found at ${HARNESS}" >&2
  exit 1
fi

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
    if [[ -n "${serial}" ]]; then
      if "${ADB}" -s "${serial}" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' | grep -q '^1$'; then
        echo "${serial}"
        return 0
      fi
    fi
    sleep 2
  done

  echo "Timed out waiting for ${avd_name} to boot." >&2
  exit 1
}

run_instrumentation() {
  local serial="$1"
  local class_name="$2"
  shift 2

  local test_class="${class_name%%#*}"
  local test_name="${class_name#*#}"
  local cmd=(
    python3
    "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${test_class}"
    --test-name "${test_name}"
    --arg "clearPackageData=false"
  )
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "-e" ]]; then
      cmd+=(--arg "$2=$3")
      shift 3
    else
      echo "Unsupported instrumentation argument sequence: $1" >&2
      return 1
    fi
  done
  "${cmd[@]}"
}

dump_harness_snapshot() {
  local serial="$1"
  local class_name="$2"
  shift 2

  echo "--- ${serial}: ${class_name} ---" >&2
  run_instrumentation "${serial}" "${class_name}" "$@" >&2 || true
}

dump_chat_if_available() {
  local serial="$1"
  local chat_id="$2"
  local label="$3"

  [[ -n "${chat_id}" ]] || return 0
  echo "--- ${serial}: chat ${label} (${chat_id}) ---" >&2
  run_instrumentation "${serial}" "to.iris.chat.RealRelayHarnessTest#report_chat_messages_from_args" \
    -e chat_id "${chat_id}" >&2 || true
}

dump_matrix_state() {
  local reason="$1"
  local role serial

  echo "Linked-device matrix debug dump: ${reason}" >&2
  echo "--- adb devices ---" >&2
  "${ADB}" devices >&2 || true
  if [[ -f "${RELAY_LOG}" ]]; then
    echo "--- relay log (${RELAY_LOG}) ---" >&2
    tail -n 200 "${RELAY_LOG}" >&2 || true
  fi

  for role in A B C; do
    case "${role}" in
      A) serial="${SERIAL_A}" ;;
      B) serial="${SERIAL_B}" ;;
      C) serial="${SERIAL_C}" ;;
      *) serial="" ;;
    esac

    [[ -n "${serial}" ]] || continue
    echo "=== Device ${role}: ${serial} ===" >&2
    if ! "${ADB}" -s "${serial}" get-state >/dev/null 2>&1; then
      echo "${serial} is not connected." >&2
      continue
    fi

    dump_harness_snapshot "${serial}" "to.iris.chat.RealRelayHarnessTest#report_logged_in_identity"
    dump_harness_snapshot "${serial}" "to.iris.chat.RealRelayHarnessTest#report_device_roster_snapshot"
    dump_chat_if_available "${serial}" "${OWNER_X_HEX:-}" "owner X"
    dump_chat_if_available "${serial}" "${OWNER_Y_HEX:-}" "owner Y"
    dump_harness_snapshot "${serial}" "to.iris.chat.RealRelayHarnessTest#report_runtime_debug_snapshot"
    dump_harness_snapshot "${serial}" "to.iris.chat.RealRelayHarnessTest#report_persisted_protocol_snapshot"
  done
}

run_matrix_step() {
  local label="$1"
  local serial="$2"
  local class_name="$3"
  local step_log
  local status
  shift 3

  step_log="$(mktemp -t iris-relay-matrix-step.XXXXXX)"
  if run_instrumentation "${serial}" "${class_name}" "$@" >"${step_log}" 2>&1; then
    cat "${step_log}"
    rm -f "${step_log}"
    return 0
  else
    status="$?"
  fi

  if [[ "${class_name}" == *"#wait_for_message_from_args" ]] &&
    grep -q '^INSTRUMENTATION_STATUS_CODE: 0$' "${step_log}" &&
    grep -Eq '^INSTRUMENTATION_STATUS: matching_count=[1-9][0-9]*$' "${step_log}" &&
    grep -q '^INSTRUMENTATION_RESULT: shortMsg=Process crashed\.$' "${step_log}"; then
    echo "${label} reported the expected message before instrumentation teardown crashed; continuing." >&2
    cat "${step_log}"
    rm -f "${step_log}"
    return 0
  fi

  echo "${label} failed on ${serial} (${class_name}) with exit code ${status}" >&2
  cat "${step_log}" >&2 || true
  rm -f "${step_log}"
  dump_matrix_state "${label}"
  exit "${status}"
}

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

# Run a harness test in the background, streaming its output into action_log.
# Sets the global IRIS_BACKGROUND_PID and writes the harness exit code into
# exit_file once the test finishes.
run_instrumentation_background() {
  local serial="$1"
  local class_name="$2"
  local action_log="$3"
  local exit_file="$4"
  shift 4

  local test_class="${class_name%%#*}"
  local test_name="${class_name#*#}"
  local cmd=(
    python3
    "${HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${test_class}"
    --test-name "${test_name}"
    --arg "clearPackageData=false"
  )
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "-e" ]]; then
      cmd+=(--arg "$2=$3")
      shift 3
    else
      echo "Unsupported instrumentation argument sequence: $1" >&2
      return 1
    fi
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
  local avd serial
  for avd in "${DEFAULT_AVDS[@]}"; do
    serial="$(ensure_avd_running "${avd}")"
    add_unique_serial "${serial}"
  done
  while IFS= read -r serial; do
    add_unique_serial "${serial}"
  done < <(connected_serials)

  if [[ ${#SELECTED_SERIALS[@]} -lt 3 ]]; then
    echo "Need three Android devices for linked-device relay matrix; found ${#SELECTED_SERIALS[@]}." >&2
    exit 1
  fi

  SERIAL_A="${SERIAL_A:-${SELECTED_SERIALS[0]}}"
  SERIAL_B="${SERIAL_B:-${SELECTED_SERIALS[1]}}"
  SERIAL_C="${SERIAL_C:-${SELECTED_SERIALS[2]}}"
}

cleanup() {
  if [[ -n "${RELAY_PID}" ]]; then
    stop_local_rust_relay "${RELAY_PID}"
  fi
}
trap cleanup EXIT

if ! assert_local_relay_healthy >/dev/null 2>&1; then
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
fi

echo "Ensuring three Android-device topology is running"
select_default_serials

for serial in "${SERIAL_A}" "${SERIAL_B}" "${SERIAL_C}"; do
  "${ADB}" -s "${serial}" reverse "tcp:$(local_relay_port)" "tcp:$(local_relay_port)" >/dev/null || true
done

echo "Installing app and test APKs"
build_android_debug_apks "$(local_android_loopback_relay_url)" "$(local_relay_set_id)" >/dev/null
install_android_debug_apks_on_serials "${ADB}" "${SERIAL_A}" "${SERIAL_B}" "${SERIAL_C}"

for serial in "${SERIAL_A}" "${SERIAL_B}" "${SERIAL_C}"; do
  echo "Clearing ${PACKAGE_NAME} on ${serial}"
  "${ADB}" -s "${serial}" shell pm clear "${PACKAGE_NAME}" >/dev/null
  "${ADB}" -s "${serial}" shell pm clear "${TEST_PACKAGE_NAME}" >/dev/null || true
done

echo "Creating owner X primary on ${SERIAL_A}"
ACCOUNT_A="$(run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#create_account_and_report_identity")"
OWNER_X_NPUB="$(printf '%s\n' "${ACCOUNT_A}" | extract_status "npub")"
OWNER_X_HEX="$(printf '%s\n' "${ACCOUNT_A}" | extract_status "public_key_hex")"

echo "Starting linked device on ${SERIAL_B}"
LINK_LOG="$(mktemp -t iris-relay-matrix-link-log.XXXXXX)"
LINK_EXIT="$(mktemp -t iris-relay-matrix-link-exit.XXXXXX)"
IRIS_BACKGROUND_PID=""
run_instrumentation_background "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#start_link_invite_and_wait_for_authorization_from_args" \
  "${LINK_LOG}" "${LINK_EXIT}" \
  -e owner_input "${OWNER_X_NPUB}" \
  -e authorization_state AUTHORIZED
LINK_PID="${IRIS_BACKGROUND_PID}"

LINK_URL="$(wait_for_status_in_file "${LINK_LOG}" invite_url 120)"

echo "Authorizing linked device on ${SERIAL_A}"
run_matrix_step "authorize linked device" "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#add_authorized_device_from_args" -e device_input "${LINK_URL}" >/dev/null

wait "${LINK_PID}" || true
LINK_STATUS="$(cat "${LINK_EXIT}")"
if [[ "${LINK_STATUS}" -ne 0 ]]; then
  echo "Linked-device authorization harness failed with exit code ${LINK_STATUS}" >&2
  cat "${LINK_LOG}" >&2 || true
  exit "${LINK_STATUS}"
fi
LINKED_B="$(cat "${LINK_LOG}")"
DEVICE_B_NPUB="$(printf '%s\n' "${LINKED_B}" | extract_status "device_npub")"
DEVICE_B_HEX="$(printf '%s\n' "${LINKED_B}" | extract_status "device_public_key_hex")"

echo "Creating owner Y peer on ${SERIAL_C}"
ACCOUNT_C="$(run_matrix_step "create owner Y peer" "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#create_account_and_report_identity")"
OWNER_Y_NPUB="$(printf '%s\n' "${ACCOUNT_C}" | extract_status "npub")"
OWNER_Y_HEX="$(printf '%s\n' "${ACCOUNT_C}" | extract_status "public_key_hex")"

echo "A sends m1 to C"
run_matrix_step "A send m1 to C" "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_Y_NPUB}" -e message "m1" >/dev/null
run_matrix_step "C wait for m1 from A" "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_X_HEX}" -e message "m1" -e direction incoming >/dev/null
run_matrix_step "B wait for A self-sync m1" "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m1" -e direction outgoing >/dev/null

echo "C replies with m2"
run_matrix_step "C send m2 to X" "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_X_NPUB}" -e message "m2" >/dev/null
run_matrix_step "A wait for m2 from C" "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m2" -e direction incoming >/dev/null
run_matrix_step "B wait for C incoming m2" "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m2" -e direction incoming >/dev/null

echo "B sends m3 to C"
run_matrix_step "B send m3 to C" "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_Y_NPUB}" -e message "m3" >/dev/null
run_matrix_step "C wait for m3 from B" "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_X_HEX}" -e message "m3" -e direction incoming >/dev/null
run_matrix_step "A wait for B self-sync m3" "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m3" -e direction outgoing >/dev/null

echo "Revoking B from the roster"
run_matrix_step "revoke B from roster" "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#remove_authorized_device_from_args" -e device_input "${DEVICE_B_HEX}" >/dev/null
run_matrix_step "B wait for revoked state" "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_revoked_state" >/dev/null

echo "Three-device relay matrix passed"
echo "A=${SERIAL_A} B=${SERIAL_B} C=${SERIAL_C}"
