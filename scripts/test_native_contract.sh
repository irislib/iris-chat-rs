#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="${ROOT_DIR}/android"
ANDROID_TEST_AVD="${IRIS_ANDROID_QA_AVD:-Medium_Phone_API_36.1}"
CONTRACT_CLASSES="to.iris.chat.core.AppManagerContractTest"
SMOKE_CLASSES="to.iris.chat.PikaLikeUiTest,to.iris.chat.account.AndroidKeystoreSecretStoreTest"

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

run_filtered_android_test() {
  local serial="$1"
  local classes="$2"

  (
    cd "${ANDROID_DIR}"
    ANDROID_SERIAL="${serial}" \
      ./gradlew \
      :app:connectedDebugAndroidTest \
      -Pandroid.testInstrumentationRunnerArguments.class="${classes}"
  )
}

if [[ "${IRIS_SKIP_FAST:-0}" != "1" ]]; then
  "${ROOT_DIR}/scripts/test_fast.sh"
fi

ANDROID_SERIAL_VALUE="$(resolve_serial)"
if [[ -z "${ANDROID_SERIAL_VALUE}" ]]; then
  echo "Failed to resolve an Android emulator serial for qa-native-contract." >&2
  exit 1
fi

run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${CONTRACT_CLASSES}"
run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${SMOKE_CLASSES}"
