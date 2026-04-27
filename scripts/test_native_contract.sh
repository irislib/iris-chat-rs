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

"${ROOT_DIR}/scripts/test_fast.sh"

ANDROID_SERIAL_VALUE="$(resolve_serial)"
if [[ -z "${ANDROID_SERIAL_VALUE}" ]]; then
  echo "Failed to resolve an Android emulator serial for qa-native-contract." >&2
  exit 1
fi

run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${CONTRACT_CLASSES}"
run_filtered_android_test "${ANDROID_SERIAL_VALUE}" "${SMOKE_CLASSES}"
