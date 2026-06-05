#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
NDK_VERSION="28.2.13676358"

find_android_sdk() {
  local sdk_dir="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "${sdk_dir}" && -f "${LOCAL_PROPERTIES}" ]]; then
    sdk_dir="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
  fi
  printf '%s' "${sdk_dir}"
}

print_status() {
  local label="$1"
  local value="$2"
  printf '%-28s %s\n' "${label}" "${value}"
}

require_cmd() {
  command -v "$1" 2>/dev/null || true
}

SDK_DIR="$(find_android_sdk)"
ADB=""
EMULATOR=""
if [[ -n "${SDK_DIR}" ]]; then
  ADB="${SDK_DIR}/platform-tools/adb"
  EMULATOR="${SDK_DIR}/emulator/emulator"
fi

CARGO_BIN="$(require_cmd cargo)"
RUSTUP_BIN="$(require_cmd rustup)"
UNIFFI_BIN="$(require_cmd uniffi-bindgen)"
PYTHON_BIN="$(require_cmd python3)"
XCODEBUILD_BIN="$(require_cmd xcodebuild)"
XCRUN_BIN="$(require_cmd xcrun)"
JUST_BIN="$(require_cmd just)"
XCODEGEN_BIN="$(require_cmd xcodegen)"

print_status "Repo root" "${ROOT_DIR}"
print_status "cargo" "${CARGO_BIN:-missing}"
print_status "rustup" "${RUSTUP_BIN:-missing}"
print_status "local uniffi-bindgen" "$( [[ -f "${ROOT_DIR}/core/uniffi-bindgen/src/main.rs" ]] && printf '%s' "${ROOT_DIR}/core/uniffi-bindgen" || printf 'missing' )"
print_status "global uniffi-bindgen" "${UNIFFI_BIN:-optional}"
print_status "python3" "${PYTHON_BIN:-missing}"
print_status "just" "${JUST_BIN:-missing}"
print_status "xcodegen" "${XCODEGEN_BIN:-missing}"
print_status "xcodebuild" "${XCODEBUILD_BIN:-missing}"
print_status "xcrun" "${XCRUN_BIN:-missing}"
print_status "Android SDK" "${SDK_DIR:-missing}"
print_status "adb" "$( [[ -x "${ADB}" ]] && printf '%s' "${ADB}" || printf 'missing' )"
print_status "emulator" "$( [[ -x "${EMULATOR}" ]] && printf '%s' "${EMULATOR}" || printf 'missing' )"
print_status "Android NDK" "$( [[ -d "${SDK_DIR}/ndk/${NDK_VERSION}" ]] && printf '%s' "${SDK_DIR}/ndk/${NDK_VERSION}" || printf 'missing' )"

if [[ -n "${RUSTUP_BIN}" ]]; then
  echo "---"
  print_status "Rust targets" "$("${RUSTUP_BIN}" target list --installed | paste -sd ',' -)"
fi

if [[ -n "${XCRUN_BIN}" ]]; then
  echo "---"
  IOS_RUNTIME_COUNT="$("${XCRUN_BIN}" simctl list runtimes available 2>/dev/null | grep -c "iOS" || true)"
  IOS_DEVICE_COUNT="$("${XCRUN_BIN}" simctl list devices available 2>/dev/null | grep -Ec "^[[:space:]]+[A-Za-z0-9]" || true)"
  print_status "Available iOS runtimes" "${IOS_RUNTIME_COUNT}"
  print_status "Available iOS devices" "${IOS_DEVICE_COUNT}"
fi

echo "---"
cat <<'EOF'
Recommended next steps:

1. Ensure Rust mobile targets are installed:
   rustup target add aarch64-linux-android aarch64-apple-ios aarch64-apple-ios-sim

2. Start Android emulators:
   ./scripts/run_android_emulators.sh

3. Create and boot iOS simulators:
   ./scripts/run_ios_simulators.sh

4. Run the fast local gate:
   just qa
EOF
