#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ios_simulator="${IRIS_CHAT_LAB_ALLOCATED_IOS_SIMULATOR:-}"
ios_device="${IRIS_CHAT_LAB_ALLOCATED_IOS_DEVICE:-}"
android_device="${IRIS_CHAT_LAB_ALLOCATED_ANDROID:-}"
if [[ -z "$ios_simulator" || -z "$ios_device" || -z "$android_device" ]]; then
  echo "managed native allocation is incomplete; run scripts/verify.sh full" >&2
  exit 75
fi
export IRIS_IOS_SIM_DESTINATION="platform=iOS Simulator,id=$ios_simulator"
export IRIS_LAN_IOS_UDID="$ios_device"
export IRIS_ANDROID_SERIAL="$android_device"
export IRIS_LAN_ANDROID_SERIAL="$android_device"

if [[ "${IRIS_NATIVE_LAB_RESET:-0}" == "1" ]]; then
  export IRIS_NATIVE_LAB_ALLOW_RESET=1
  "$ROOT/scripts/native_state_reset.sh" ios-simulator \
    --udid "$ios_simulator" \
    --erase

  "$ROOT/scripts/native_state_reset.sh" android \
    --serial "$android_device" \
    --bundle-id "${IRIS_CHAT_ANDROID_PACKAGE:-to.iris.chat.debug}" \
    --test-bundle-id "${IRIS_CHAT_ANDROID_TEST_PACKAGE:-to.iris.chat.test}"
fi

"$ROOT/scripts/test-all-platforms"

gate_args=(--full --on-device --skip-fast)
if [[ "${IRIS_VERIFY_FULL_RELIABILITY:-1}" == "1" ]]; then
  gate_args+=(--reliability-lab)
fi
if [[ "${IRIS_VERIFY_FULL_MACOS_VM:-0}" == "1" ]]; then
  gate_args+=(--macos-vm-e2e)
fi
exec "$ROOT/scripts/test-release-gate" "${gate_args[@]}"
