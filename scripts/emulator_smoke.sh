#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
if [[ ! -x "${ADB}" ]]; then
  echo "adb not found at ${ADB}" >&2
  exit 1
fi

PACKAGE_NAME="to.iris.chat"
MAIN_ACTIVITY="to.iris.chat.MainActivity"
APK_PATH="${ROOT_DIR}/android/app/build/outputs/apk/debug/app-debug.apk"
DEFAULT_DEVICES=("emulator-5554" "emulator-5556")

usage() {
  cat <<'EOF'
Usage: scripts/emulator_smoke.sh [--clear] [device...]

Build expectations:
  - Run just android-assemble first, or let this script fail if the APK is missing.

Behavior:
  - Verifies each requested emulator is online.
  - Installs the same debug APK to each device.
  - Optionally clears app data before launch with --clear.
  - Launches the main activity on each device.
EOF
}

CLEAR_STATE=0
DEVICES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --clear)
      CLEAR_STATE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      DEVICES+=("$1")
      shift
      ;;
  esac
done

if [[ ${#DEVICES[@]} -eq 0 ]]; then
  DEVICES=("${DEFAULT_DEVICES[@]}")
fi

if [[ ! -f "${APK_PATH}" ]]; then
  echo "Debug APK missing at ${APK_PATH}. Run just android-assemble first." >&2
  exit 1
fi

for device in "${DEVICES[@]}"; do
  "${ADB}" -s "${device}" get-state >/dev/null 2>&1 || {
    echo "Device ${device} is not online." >&2
    exit 1
  }

  echo "Installing ${PACKAGE_NAME} on ${device}"
  "${ADB}" -s "${device}" install -r "${APK_PATH}" >/dev/null

  if [[ ${CLEAR_STATE} -eq 1 ]]; then
    echo "Clearing app data on ${device}"
    "${ADB}" -s "${device}" shell pm clear "${PACKAGE_NAME}" >/dev/null
  fi

  if ! "${ADB}" -s "${device}" shell pm list packages "${PACKAGE_NAME}" | grep -q "${PACKAGE_NAME}"; then
    echo "Package ${PACKAGE_NAME} was not installed on ${device}." >&2
    exit 1
  fi

  echo "Launching ${PACKAGE_NAME} on ${device}"
  "${ADB}" -s "${device}" shell am start -n "${PACKAGE_NAME}/${MAIN_ACTIVITY}" >/dev/null
done

echo "Smoke setup complete for: ${DEVICES[*]}"
