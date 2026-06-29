#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"
LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
ANDROID_HARNESS="${ROOT_DIR}/scripts/run_harness.py"
IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
MACOS_HARNESS="${ROOT_DIR}/scripts/run_macos_harness.py"

ANDROID_RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS="to.iris.chat.RealRelayHarnessTest"
ANDROID_APP_PACKAGE="${ANDROID_APP_PACKAGE:-to.iris.chat.debug}"
ANDROID_TEST_PACKAGE="${ANDROID_TEST_PACKAGE:-to.iris.chat.test}"

INCLUDE_ANDROID=1
INCLUDE_IOS=1
INCLUDE_MACOS=1
ALLOW_SKIP=0
ON_DEVICE=0
CLEAR_STATE=1
REBUILD=1
TIMEOUT_MS="${IRIS_LAN_TIMEOUT_MS:-60000}"
HOLD_MS="${IRIS_LAN_HOLD_MS:-15000}"

ANDROID_SERIAL="${IRIS_LAN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"
IOS_UDID="${IRIS_LAN_IOS_UDID:-${IOS_UDID:-}}"
IOS_SIMULATOR="${IRIS_LAN_IOS_SIMULATOR:-Iris Chat iPhone}"
IOS_RUN_ID="${IRIS_LAN_IOS_RUN_ID:-lan-ios}"
MACOS_RUN_ID="${IRIS_LAN_MACOS_RUN_ID:-lan-macos}"
ANDROID_LABEL="android"
IOS_LABEL="iphone"
MACOS_LABEL="mac"
IOS_IS_SIMULATOR=0

cleanup() {
  local exit_code=$?
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" != "1" && "${IOS_IS_SIMULATOR}" -eq 1 ]]; then
    iris_e2e_shutdown_ios_simulators "${IOS_UDID}"
  fi
  exit "${exit_code}"
}
trap cleanup EXIT

usage() {
  cat <<EOF
usage: ./scripts/nearby_lan_visibility_matrix.sh [options]

Checks that clients on the same Wi-Fi/LAN can discover each other through the
LAN-only nearby transport.

Options:
  --android <serial>       Use this Android adb serial
  --ios-udid <udid>        Use this iPhone/iOS simulator UDID
  --ios-simulator <name>   Boot/use this simulator if no UDID is set
  --on-device              Require Android + iPhone + Mac participants
  --no-android             Skip Android
  --no-ios                 Skip iOS/iPhone
  --no-macos               Skip macOS
  --allow-skip             Exit 0 if fewer than two participants are available
  --no-clear               Keep existing harness state
  --no-rebuild             Reuse existing Android/iOS/macOS test builds
  --timeout-ms <ms>        Peer wait timeout. Default: ${TIMEOUT_MS}
  --hold-ms <ms>           Keep LAN visible after success. Default: ${HOLD_MS}
  -h, --help               Show this help

Environment:
  IRIS_LAN_ANDROID_SERIAL, IRIS_LAN_IOS_UDID, IRIS_LAN_IOS_SIMULATOR,
  IRIS_LAN_TIMEOUT_MS, IRIS_LAN_HOLD_MS
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --android)
      ANDROID_SERIAL="$2"
      shift 2
      ;;
    --ios-udid|--iphone)
      IOS_UDID="$2"
      shift 2
      ;;
    --ios-simulator)
      IOS_SIMULATOR="$2"
      shift 2
      ;;
    --on-device)
      ON_DEVICE=1
      shift
      ;;
    --no-android)
      INCLUDE_ANDROID=0
      shift
      ;;
    --no-ios)
      INCLUDE_IOS=0
      shift
      ;;
    --no-macos)
      INCLUDE_MACOS=0
      shift
      ;;
    --allow-skip)
      ALLOW_SKIP=1
      shift
      ;;
    --no-clear)
      CLEAR_STATE=0
      shift
      ;;
    --no-rebuild)
      REBUILD=0
      shift
      ;;
    --timeout-ms)
      TIMEOUT_MS="$2"
      shift 2
      ;;
    --hold-ms)
      HOLD_MS="$2"
      shift 2
      ;;
    -h|--help|help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${ON_DEVICE}" -eq 1 ]]; then
  INCLUDE_ANDROID=1
  INCLUDE_IOS=1
  INCLUDE_MACOS=1
fi

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

require_value() {
  local name="$1"
  local value="$2"
  if [[ -z "${value}" ]]; then
    echo "Missing required status value: ${name}" >&2
    return 1
  fi
}

require_harness_success() {
  local label="$1"
  local output="$2"
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "${label} did not report harness success." >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_STATUS_CODE: -2$|^FAILURES!!!$|^Process crashed\\.$'; then
    echo "${label} reported a failed instrumentation test." >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
}

android_sdk_dir() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "${sdk}" && -f "${LOCAL_PROPERTIES}" ]]; then
    sdk="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
  fi
  printf '%s\n' "${sdk}"
}

first_physical_android_serial() {
  local adb="$1"
  "${adb}" devices -l 2>/dev/null |
    awk 'NR > 1 && $2 == "device" && $1 !~ /^emulator-/ { print $1; exit }'
}

first_ios_device_udid() {
  if ! command -v xcrun >/dev/null 2>&1; then
    return 0
  fi
  xcrun xctrace list devices 2>/dev/null |
    awk '
      /^== Simulators ==/ { in_simulators = 1; next }
      /^== Devices ==/ { in_simulators = 0; next }
      in_simulators { next }
      /(iPhone|iPad)/ {
        if (match($0, /\([0-9A-Fa-f-]{24,}\)/)) {
          value = substr($0, RSTART + 1, RLENGTH - 2)
          print value
          exit
        }
      }
    '
}

extract_ios_udid() {
  printf '%s\n' "$1" | sed -nE 's/.* ([0-9A-F-]{36}) .*/\1/p' | tail -n 1
}

SDK_DIR=""
ADB=""
if [[ "${INCLUDE_ANDROID}" -eq 1 ]]; then
  SDK_DIR="$(android_sdk_dir)"
  if [[ -n "${SDK_DIR}" ]]; then
    ADB="${SDK_DIR}/platform-tools/adb"
  fi
  if [[ -z "${ANDROID_SERIAL}" && -n "${ADB}" && -x "${ADB}" ]]; then
    ANDROID_SERIAL="$(first_physical_android_serial "${ADB}")"
  fi
fi

if [[ "${INCLUDE_IOS}" -eq 1 && -z "${IOS_UDID}" ]]; then
  IOS_UDID="$(first_ios_device_udid)"
fi

if [[ "${INCLUDE_IOS}" -eq 1 && -z "${IOS_UDID}" && "${ON_DEVICE}" -eq 0 ]]; then
  if command -v xcrun >/dev/null 2>&1; then
    ios_boot_output="$("${ROOT_DIR}/scripts/run_ios_simulators.sh" --no-open "${IOS_SIMULATOR}")"
    IOS_UDID="$(extract_ios_udid "${ios_boot_output}")"
    IOS_IS_SIMULATOR=1
  fi
fi

PARTICIPANT_TYPES=()
PARTICIPANT_LABELS=()
PARTICIPANT_IDS=()
PARTICIPANT_RUN_IDS=()
PARTICIPANT_NPUBS=()
PARTICIPANT_HEXS=()

add_participant() {
  PARTICIPANT_TYPES+=("$1")
  PARTICIPANT_LABELS+=("$2")
  PARTICIPANT_IDS+=("$3")
  PARTICIPANT_RUN_IDS+=("$4")
}

if [[ "${INCLUDE_ANDROID}" -eq 1 && -n "${ANDROID_SERIAL}" ]]; then
  if [[ -z "${ADB}" || ! -x "${ADB}" ]]; then
    echo "adb not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or android/local.properties." >&2
    exit 1
  fi
  add_participant android "${ANDROID_LABEL}" "${ANDROID_SERIAL}" ""
elif [[ "${ON_DEVICE}" -eq 1 && "${INCLUDE_ANDROID}" -eq 1 ]]; then
  echo "No physical Android device found. Set IRIS_LAN_ANDROID_SERIAL or ANDROID_SERIAL." >&2
  exit 1
fi

if [[ "${INCLUDE_IOS}" -eq 1 && -n "${IOS_UDID}" ]]; then
  add_participant ios "${IOS_LABEL}" "${IOS_UDID}" "${IOS_RUN_ID}"
elif [[ "${ON_DEVICE}" -eq 1 && "${INCLUDE_IOS}" -eq 1 ]]; then
  echo "No iPhone UDID found. Set IRIS_LAN_IOS_UDID." >&2
  exit 1
fi

if [[ "${INCLUDE_MACOS}" -eq 1 && "$(uname -s)" == "Darwin" ]]; then
  add_participant macos "${MACOS_LABEL}" "macos" "${MACOS_RUN_ID}"
elif [[ "${ON_DEVICE}" -eq 1 && "${INCLUDE_MACOS}" -eq 1 ]]; then
  echo "macOS participant requires a Darwin host." >&2
  exit 1
fi

if [[ "${#PARTICIPANT_TYPES[@]}" -lt 2 ]]; then
  echo "Need at least two LAN participants; found ${#PARTICIPANT_TYPES[@]}." >&2
  if [[ "${ALLOW_SKIP}" -eq 1 ]]; then
    echo "Skipping LAN visibility matrix."
    exit 0
  fi
  exit 1
fi

run_android_test() {
  local serial="$1"
  local test_name="$2"
  shift 2
  local cmd=(
    python3 "${ANDROID_HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${ANDROID_RUNNER}"
    --class-name "${ANDROID_CLASS}"
    --test-name "${test_name}"
    --arg "clearPackageData=false"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" </dev/null 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  require_harness_success "Android ${test_name} on ${serial}" "${output}"
  printf '%s\n' "${output}"
}

run_ios_test() {
  local udid="$1"
  local run_id="$2"
  local action="$3"
  shift 3
  local cmd=(python3 "${IOS_HARNESS}" --udid "${udid}" --run-id "${run_id}" --action "${action}")
  if [[ "${CLEAR_STATE}" -eq 1 && "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--reset)
  fi
  if [[ "${REBUILD}" -eq 1 && "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--rebuild)
  fi
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" </dev/null 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  require_harness_success "iOS ${action} on ${udid}" "${output}"
  printf '%s\n' "${output}"
}

run_macos_test() {
  local run_id="$1"
  local action="$2"
  shift 2
  local cmd=(python3 "${MACOS_HARNESS}" --run-id "${run_id}" --action "${action}")
  if [[ "${CLEAR_STATE}" -eq 1 && "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--reset)
  fi
  if [[ "${REBUILD}" -eq 1 && "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--rebuild)
  fi
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" </dev/null 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  require_harness_success "macOS ${action}" "${output}"
  printf '%s\n' "${output}"
}

if [[ "${INCLUDE_ANDROID}" -eq 1 && -n "${ANDROID_SERIAL}" ]]; then
  if [[ "${REBUILD}" -eq 1 ]]; then
    echo "Building Android debug/test APKs for ${ANDROID_SERIAL}"
    (cd "${ROOT_DIR}/android" && ANDROID_SERIAL="${ANDROID_SERIAL}" ./gradlew :app:installDebug :app:installDebugAndroidTest)
  fi
  if [[ "${CLEAR_STATE}" -eq 1 ]]; then
    "${ADB}" -s "${ANDROID_SERIAL}" shell pm clear "${ANDROID_APP_PACKAGE}" >/dev/null || true
    "${ADB}" -s "${ANDROID_SERIAL}" shell pm clear "${ANDROID_TEST_PACKAGE}" >/dev/null || true
  fi
  "${ADB}" -s "${ANDROID_SERIAL}" shell pm grant "${ANDROID_APP_PACKAGE}" android.permission.NEARBY_WIFI_DEVICES >/dev/null 2>&1 || true
fi

echo "LAN participants:"
for index in "${!PARTICIPANT_TYPES[@]}"; do
  echo "  - ${PARTICIPANT_LABELS[$index]} (${PARTICIPANT_TYPES[$index]} ${PARTICIPANT_IDS[$index]})"
done

for index in "${!PARTICIPANT_TYPES[@]}"; do
  type="${PARTICIPANT_TYPES[$index]}"
  label="${PARTICIPANT_LABELS[$index]}"
  id="${PARTICIPANT_IDS[$index]}"
  run_id="${PARTICIPANT_RUN_IDS[$index]}"
  echo "Creating identity for ${label}"
  case "${type}" in
    android)
      output="$(run_android_test "${id}" create_account_and_report_identity)"
      ;;
    ios)
      output="$(run_ios_test "${id}" "${run_id}" create_account_and_report_identity)"
      if [[ "${IOS_IS_SIMULATOR}" -eq 1 ]]; then
        xcrun simctl privacy "${id}" grant local-network fi.siriusbusiness.irischat >/dev/null 2>&1 || true
      fi
      ;;
    macos)
      output="$(run_macos_test "${run_id}" create_account_and_report_identity)"
      ;;
    *)
      echo "Unknown participant type: ${type}" >&2
      exit 1
      ;;
  esac
  npub="$(printf '%s\n' "${output}" | extract_status npub)"
  hex="$(printf '%s\n' "${output}" | extract_status public_key_hex)"
  require_value "${label}.npub" "${npub}"
  require_value "${label}.public_key_hex" "${hex}"
  PARTICIPANT_NPUBS+=("${npub}")
  PARTICIPANT_HEXS+=("${hex}")
done

run_lan_wait() {
  local index="$1"
  local peer_input="$2"
  type="${PARTICIPANT_TYPES[$index]}"
  label="${PARTICIPANT_LABELS[$index]}"
  id="${PARTICIPANT_IDS[$index]}"
  run_id="${PARTICIPANT_RUN_IDS[$index]}"
  case "${type}" in
    android)
      run_android_test "${id}" wait_for_lan_nearby_peer_profile_from_args \
        peer_input "${peer_input}" timeout_ms "${TIMEOUT_MS}" hold_ms "${HOLD_MS}"
      ;;
    ios)
      run_ios_test "${id}" "${run_id}" wait_for_lan_nearby_peer_profile_from_args \
        peer_input "${peer_input}" timeout_ms "${TIMEOUT_MS}" hold_ms "${HOLD_MS}"
      ;;
    macos)
      run_macos_test "${run_id}" wait_for_lan_nearby_peer_profile_from_args \
        peer_input "${peer_input}" timeout_ms "${TIMEOUT_MS}" hold_ms "${HOLD_MS}"
      ;;
  esac
}

echo "Waiting for LAN peer visibility"
PIDS=()
OUTS=()
for index in "${!PARTICIPANT_TYPES[@]}"; do
  peer_index=$(( (index + 1) % ${#PARTICIPANT_TYPES[@]} ))
  output_file="$(mktemp "${TMPDIR:-/tmp}/iris-lan-${PARTICIPANT_LABELS[$index]}.XXXXXX")"
  OUTS+=("${output_file}")
  (
    echo "${PARTICIPANT_LABELS[$index]} waiting for ${PARTICIPANT_LABELS[$peer_index]}"
    run_lan_wait "${index}" "${PARTICIPANT_NPUBS[$peer_index]}"
  ) >"${output_file}" 2>&1 &
  PIDS+=("$!")
done

FAILED=0
for index in "${!PIDS[@]}"; do
  if ! wait "${PIDS[$index]}"; then
    FAILED=1
    echo "LAN wait failed for ${PARTICIPANT_LABELS[$index]}" >&2
    cat "${OUTS[$index]}" >&2
  fi
done

if [[ "${FAILED}" -ne 0 ]]; then
  exit 1
fi

for output_file in "${OUTS[@]}"; do
  cat "${output_file}"
  rm -f "${output_file}"
done

echo "LAN visibility matrix passed"
for index in "${!PARTICIPANT_TYPES[@]}"; do
  echo "${PARTICIPANT_LABELS[$index]}=${PARTICIPANT_HEXS[$index]}"
done
