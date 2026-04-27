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
PACKAGE_NAME="to.iris.chat"
DEFAULT_AVDS=("Pixel_9a" "Medium_Phone_API_36.1" "Pixel_Fold")

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

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

assert_local_relay_healthy

echo "Ensuring three emulator topology is running"
SERIAL_A="$(ensure_avd_running "${DEFAULT_AVDS[0]}")"
SERIAL_B="$(ensure_avd_running "${DEFAULT_AVDS[1]}")"
SERIAL_C="$(ensure_avd_running "${DEFAULT_AVDS[2]}")"

echo "Installing app and test APKs"
(cd "${ROOT_DIR}/android" && ./gradlew :app:installDebug :app:installDebugAndroidTest >/dev/null)

for serial in "${SERIAL_A}" "${SERIAL_B}" "${SERIAL_C}"; do
  echo "Clearing ${PACKAGE_NAME} on ${serial}"
  "${ADB}" -s "${serial}" shell pm clear "${PACKAGE_NAME}" >/dev/null
done

echo "Creating owner X primary on ${SERIAL_A}"
ACCOUNT_A="$(run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#create_account_and_report_identity")"
OWNER_X_NPUB="$(printf '%s\n' "${ACCOUNT_A}" | extract_status "npub")"
OWNER_X_HEX="$(printf '%s\n' "${ACCOUNT_A}" | extract_status "public_key_hex")"

echo "Starting linked device on ${SERIAL_B}"
LINKED_B="$(run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#start_linked_device_and_report_identity" -e owner_input "${OWNER_X_NPUB}")"
DEVICE_B_NPUB="$(printf '%s\n' "${LINKED_B}" | extract_status "device_npub")"
DEVICE_B_HEX="$(printf '%s\n' "${LINKED_B}" | extract_status "device_public_key_hex")"

echo "Authorizing linked device on ${SERIAL_A}"
run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#add_authorized_device_from_args" -e device_input "${DEVICE_B_NPUB}" >/dev/null
run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_authorization_state_from_args" -e authorization_state AUTHORIZED >/dev/null

echo "Creating owner Y peer on ${SERIAL_C}"
ACCOUNT_C="$(run_instrumentation "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#create_account_and_report_identity")"
OWNER_Y_NPUB="$(printf '%s\n' "${ACCOUNT_C}" | extract_status "npub")"
OWNER_Y_HEX="$(printf '%s\n' "${ACCOUNT_C}" | extract_status "public_key_hex")"

echo "A sends m1 to C"
run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_Y_NPUB}" -e message "m1" >/dev/null
run_instrumentation "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_X_HEX}" -e message "m1" -e direction incoming >/dev/null
run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m1" -e direction outgoing >/dev/null

echo "C replies with m2"
run_instrumentation "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_X_NPUB}" -e message "m2" >/dev/null
run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m2" -e direction incoming >/dev/null
run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m2" -e direction incoming >/dev/null

echo "B sends m3 to C"
run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#send_message_from_args" -e peer_input "${OWNER_Y_NPUB}" -e message "m3" >/dev/null
run_instrumentation "${SERIAL_C}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_X_HEX}" -e message "m3" -e direction incoming >/dev/null
run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#wait_for_message_from_args" -e chat_id "${OWNER_Y_HEX}" -e message "m3" -e direction outgoing >/dev/null

echo "Revoking B from the roster"
run_instrumentation "${SERIAL_A}" "to.iris.chat.RealRelayHarnessTest#remove_authorized_device_from_args" -e device_input "${DEVICE_B_HEX}" >/dev/null
run_instrumentation "${SERIAL_B}" "to.iris.chat.RealRelayHarnessTest#wait_for_revoked_state" >/dev/null

echo "Three-device relay matrix passed"
echo "A=${SERIAL_A} B=${SERIAL_B} C=${SERIAL_C}"
