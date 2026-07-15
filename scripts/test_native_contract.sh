#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"
ANDROID_DIR="${ROOT_DIR}/android"
ANDROID_TEST_AVD="${IRIS_ANDROID_QA_AVD:-Medium_Phone_API_36.1}"
PACKAGE_NAME="to.iris.chat.debug"
TEST_PACKAGE_NAME="${ANDROID_TEST_PACKAGE_NAME:-to.iris.chat.test}"
CONTRACT_CLASSES="to.iris.chat.core.AppManagerContractTest"
SMOKE_CLASSES="to.iris.chat.IrisChatUiSmokeTest,to.iris.chat.account.AndroidKeystoreSecretStoreTest"

resolve_serial() {
  if [[ -n "${IRIS_ANDROID_SERIAL:-}" ]]; then
    printf '%s\n' "${IRIS_ANDROID_SERIAL}"
    return 0
  fi
  if [[ -n "${ANDROID_SERIAL:-}" ]]; then
    printf '%s\n' "${ANDROID_SERIAL}"
    return 0
  fi

  local sdk_dir adb_path attached_serial
  sdk_dir="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "${sdk_dir}" && -f "${ANDROID_DIR}/local.properties" ]]; then
    sdk_dir="$(sed -n 's/^sdk\.dir=//p' "${ANDROID_DIR}/local.properties" | tail -n 1)"
  fi
  adb_path="${sdk_dir}/platform-tools/adb"
  if [[ -n "${sdk_dir}" && -x "${adb_path}" ]]; then
    attached_serial="$("${adb_path}" devices -l 2>/dev/null |
      awk 'NR > 1 && $2 == "device" && $1 !~ /^emulator-/ { print $1; exit }')"
    if [[ -n "${attached_serial}" ]]; then
      printf '%s\n' "${attached_serial}"
      return 0
    fi
  fi

  local boot_output
  boot_output="$("${ROOT_DIR}/scripts/run_android_emulators.sh" "${ANDROID_TEST_AVD}")"
  printf '%s\n' "${boot_output}" | awk 'NR == 1 { print $2 }'
}

android_sdk_dir() {
  local sdk_dir
  sdk_dir="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "${sdk_dir}" && -f "${ANDROID_DIR}/local.properties" ]]; then
    sdk_dir="$(sed -n 's/^sdk\.dir=//p' "${ANDROID_DIR}/local.properties" | tail -n 1)"
  fi
  printf '%s\n' "${sdk_dir}"
}

reset_android_app_state() {
  local serial="$1"
  local sdk_dir adb_path
  sdk_dir="$(android_sdk_dir)"
  adb_path="${sdk_dir}/platform-tools/adb"
  if [[ -z "${sdk_dir}" || ! -x "${adb_path}" ]]; then
    return 0
  fi

  "${adb_path}" -s "${serial}" shell am force-stop "${PACKAGE_NAME}" >/dev/null 2>&1 || true
  "${adb_path}" -s "${serial}" shell am force-stop "${TEST_PACKAGE_NAME}" >/dev/null 2>&1 || true
  "${adb_path}" -s "${serial}" shell pm clear "${PACKAGE_NAME}" >/dev/null 2>&1 || true
  "${adb_path}" -s "${serial}" shell pm clear "${TEST_PACKAGE_NAME}" >/dev/null 2>&1 || true
}

remove_android_relay_reverse() {
  local serial="$1"
  local sdk_dir adb_path
  sdk_dir="$(android_sdk_dir)"
  adb_path="${sdk_dir}/platform-tools/adb"
  if [[ -n "${sdk_dir}" && -x "${adb_path}" ]]; then
    "${adb_path}" -s "${serial}" reverse --remove "tcp:$(local_relay_port)" \
      >/dev/null 2>&1 || true
  fi
}

run_filtered_android_test() {
  local serial="$1"
  local classes="$2"

  reset_android_app_state "${serial}"
  if ! (
    cd "${ANDROID_DIR}"
    ANDROID_SERIAL="${serial}" \
      ./gradlew \
      :app:connectedDebugAndroidTest \
      -Pandroid.testInstrumentationRunnerArguments.class="${classes}"
  ); then
    echo "Android instrumentation failed for ${classes}; resetting app state and retrying once." >&2
    reset_android_app_state "${serial}"
    (
      cd "${ANDROID_DIR}"
      ANDROID_SERIAL="${serial}" \
        ./gradlew \
        :app:connectedDebugAndroidTest \
        -Pandroid.testInstrumentationRunnerArguments.class="${classes}"
    )
  fi
  reset_android_app_state "${serial}"
}

if [[ "${IRIS_SKIP_FAST:-0}" != "1" ]]; then
  "${ROOT_DIR}/scripts/test_fast.sh"
fi

ANDROID_SERIAL_VALUE="$(resolve_serial)"
if [[ -z "${ANDROID_SERIAL_VALUE}" ]]; then
  echo "Failed to resolve an Android emulator serial for qa-native-contract." >&2
  exit 1
fi

if [[ -z "${IRIS_LOCAL_RELAY_PORT:-}" ]]; then
  IRIS_LOCAL_RELAY_PORT="$(python3 - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
  export IRIS_LOCAL_RELAY_PORT
fi
RELAY_LOG="${TMPDIR:-/tmp}/iris-chat-native-contract-relay-${IRIS_LOCAL_RELAY_PORT}.log"
RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
trap 'remove_android_relay_reverse "${ANDROID_SERIAL_VALUE}"; stop_local_rust_relay "${RELAY_PID}"' EXIT

SDK_DIR="$(android_sdk_dir)"
ADB_PATH="${SDK_DIR}/platform-tools/adb"
"${ADB_PATH}" -s "${ANDROID_SERIAL_VALUE}" reverse \
  "tcp:$(local_relay_port)" "tcp:$(local_relay_port)"
export IRIS_DEBUG_RELAYS="$(local_android_loopback_relay_url)"
export IRIS_DEVICE_APPROVAL_RELAY_URL="${IRIS_DEBUG_RELAYS}"
export IRIS_DEBUG_RELAY_SET_ID="$(local_relay_set_id)"

run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${CONTRACT_CLASSES}"
run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${SMOKE_CLASSES}"
