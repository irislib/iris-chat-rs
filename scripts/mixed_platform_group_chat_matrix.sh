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
ANDROID_HARNESS="${ROOT_DIR}/scripts/run_harness.py"
IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
ANDROID_RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS="to.iris.chat.RealRelayHarnessTest"
ANDROID_APP_PACKAGE="to.iris.chat.debug"
ANDROID_TEST_PACKAGE="to.iris.chat.test"

ANDROID_ADMIN_AVD="${ANDROID_ADMIN_AVD:-Medium_Phone_API_36.1}"
ANDROID_MEMBER_AVD="${ANDROID_MEMBER_AVD:-Pixel_Fold}"
IOS_PRIMARY_SIM="${IOS_PRIMARY_SIM:-Iris Chat iPhone}"
IOS_MEMBER_SIM="${IOS_MEMBER_SIM:-Iris Chat iPhone 2}"
ANDROID_ADMIN_SERIAL="${ANDROID_ADMIN_SERIAL:-}"
ANDROID_MEMBER_SERIAL="${ANDROID_MEMBER_SERIAL:-}"
IOS_PRIMARY_UDID="${IOS_PRIMARY_UDID:-}"
IOS_MEMBER_UDID="${IOS_MEMBER_UDID:-}"
IOS_PRIMARY_RUN_ID="${IOS_PRIMARY_RUN_ID:-ios-primary}"
IOS_MEMBER_RUN_ID="${IOS_MEMBER_RUN_ID:-ios-member}"
AM_USER="${AM_USER:-0}"
CLEAR_STATE=1
PHASE="${PHASE:-all}"
STATE_FILE="${STATE_FILE:-/tmp/iris-mixed-platform-state.env}"
SKIP_BUILD=0
USE_EXISTING_RELAY=0

RELAY_LOG="${RELAY_LOG:-/tmp/ndr-mixed-platform-relay.log}"
ANDROID_CREATOR_GROUP_NAME="${ANDROID_CREATOR_GROUP_NAME:-AndroidCreatorGroup}"
IOS_CREATOR_GROUP_NAME="${IOS_CREATOR_GROUP_NAME:-IosCreatorGroup}"

usage() {
  cat <<EOF
Usage: scripts/mixed_platform_group_chat_matrix.sh [options]

Options:
  --android-admin-avd NAME     Android creator/member AVD. Default: ${ANDROID_ADMIN_AVD}
  --android-member-avd NAME    Android second-device AVD. Default: ${ANDROID_MEMBER_AVD}
  --ios-primary NAME           First iOS simulator name. Default: ${IOS_PRIMARY_SIM}
  --ios-member NAME            Second iOS simulator name. Default: ${IOS_MEMBER_SIM}
  --no-clear                   Keep existing harness state instead of resetting first
  --phase PHASE                all, android-owned, or ios-owned. Default: ${PHASE}
  --state-file PATH            Phase state file. Default: ${STATE_FILE}
  --skip-build                 Reuse already-installed Android/iOS artifacts
  --use-existing-relay         Require an already-running local relay and do not stop it
  -h, --help                   Show this help

Environment overrides:
  ANDROID_ADMIN_SERIAL, ANDROID_MEMBER_SERIAL, IOS_PRIMARY_UDID, IOS_MEMBER_UDID,
  IOS_PRIMARY_RUN_ID, IOS_MEMBER_RUN_ID, RELAY_LOG, PHASE, STATE_FILE,
  ANDROID_CREATOR_GROUP_NAME, IOS_CREATOR_GROUP_NAME
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --android-admin-avd)
      ANDROID_ADMIN_AVD="$2"
      shift 2
      ;;
    --android-member-avd)
      ANDROID_MEMBER_AVD="$2"
      shift 2
      ;;
    --ios-primary)
      IOS_PRIMARY_SIM="$2"
      shift 2
      ;;
    --ios-member)
      IOS_MEMBER_SIM="$2"
      shift 2
      ;;
    --no-clear)
      CLEAR_STATE=0
      shift
      ;;
    --phase)
      PHASE="$2"
      shift 2
      ;;
    --state-file)
      STATE_FILE="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --use-existing-relay)
      USE_EXISTING_RELAY=1
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

case "${PHASE}" in
  all|android-owned|ios-owned)
    ;;
  *)
    echo "Unknown phase: ${PHASE}" >&2
    usage >&2
    exit 1
    ;;
esac

if [[ "${PHASE}" == "ios-owned" ]]; then
  CLEAR_STATE=0
fi

extract_status() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

extract_ios_udid() {
  printf '%s\n' "$1" | sed -nE 's/.* ([0-9A-F-]{36}) .*/\1/p' | tail -n 1
}

require_value() {
  local name="$1"
  local value="$2"
  if [[ -z "${value}" ]]; then
    echo "Missing required status value: ${name}" >&2
    return 1
  fi
}

write_state() {
  mkdir -p "$(dirname "${STATE_FILE}")"
  {
    printf 'ANDROID_ADMIN_SERIAL=%q\n' "${ANDROID_ADMIN_SERIAL}"
    printf 'ANDROID_MEMBER_SERIAL=%q\n' "${ANDROID_MEMBER_SERIAL}"
    printf 'IOS_PRIMARY_UDID=%q\n' "${IOS_PRIMARY_UDID}"
    printf 'IOS_MEMBER_UDID=%q\n' "${IOS_MEMBER_UDID}"
    printf 'IOS_PRIMARY_RUN_ID=%q\n' "${IOS_PRIMARY_RUN_ID}"
    printf 'IOS_MEMBER_RUN_ID=%q\n' "${IOS_MEMBER_RUN_ID}"
    printf 'ANDROID_ADMIN_NPUB=%q\n' "${ANDROID_ADMIN_NPUB:-}"
    printf 'ANDROID_ADMIN_HEX=%q\n' "${ANDROID_ADMIN_HEX:-}"
    printf 'ANDROID_MEMBER_NPUB=%q\n' "${ANDROID_MEMBER_NPUB:-}"
    printf 'ANDROID_MEMBER_HEX=%q\n' "${ANDROID_MEMBER_HEX:-}"
    printf 'IOS_PRIMARY_NPUB=%q\n' "${IOS_PRIMARY_NPUB:-}"
    printf 'IOS_PRIMARY_HEX=%q\n' "${IOS_PRIMARY_HEX:-}"
    printf 'IOS_MEMBER_NPUB=%q\n' "${IOS_MEMBER_NPUB:-}"
    printf 'IOS_MEMBER_HEX=%q\n' "${IOS_MEMBER_HEX:-}"
    printf 'ANDROID_GROUP_CHAT_ID=%q\n' "${ANDROID_GROUP_CHAT_ID:-}"
    printf 'IOS_GROUP_CHAT_ID=%q\n' "${IOS_GROUP_CHAT_ID:-}"
  } >"${STATE_FILE}"
}

load_state() {
  if [[ ! -f "${STATE_FILE}" ]]; then
    echo "Phase state file not found: ${STATE_FILE}" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "${STATE_FILE}"
  require_value ANDROID_ADMIN_NPUB "${ANDROID_ADMIN_NPUB:-}"
  require_value ANDROID_ADMIN_HEX "${ANDROID_ADMIN_HEX:-}"
  require_value ANDROID_MEMBER_NPUB "${ANDROID_MEMBER_NPUB:-}"
  require_value ANDROID_MEMBER_HEX "${ANDROID_MEMBER_HEX:-}"
  require_value IOS_PRIMARY_NPUB "${IOS_PRIMARY_NPUB:-}"
  require_value IOS_PRIMARY_HEX "${IOS_PRIMARY_HEX:-}"
  require_value IOS_MEMBER_NPUB "${IOS_MEMBER_NPUB:-}"
  require_value IOS_MEMBER_HEX "${IOS_MEMBER_HEX:-}"
}

run_android_test() {
  local serial="$1"
  local test_name="$2"
  shift 2

  local cmd=(
    python3
    "${ANDROID_HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${ANDROID_RUNNER}"
    --class-name "${ANDROID_CLASS}"
    --test-name "${test_name}"
    --user "${AM_USER}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done

  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  printf '%s\n' "${output}"
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "Android harness ${test_name} did not report success on ${serial}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
}

run_ios_test() {
  local udid="$1"
  local run_id="$2"
  local action="$3"
  shift 3

  local cmd=(
    python3
    "${IOS_HARNESS}"
    --udid "${udid}"
    --run-id "${run_id}"
    --action "${action}"
  )
  if [[ "${CLEAR_STATE}" -eq 1 && "${action}" == "create_account_and_report_identity" ]]; then
    cmd+=(--reset)
  fi
  if [[ "${action}" == "create_account_and_report_identity" && "${run_id}" == "${IOS_PRIMARY_RUN_ID}" ]]; then
    cmd+=(--rebuild)
  fi
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done

  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "${output}" >&2
    return 1
  }
  printf '%s\n' "${output}"
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "iOS harness ${action} did not report success on ${udid}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
}

report_android_debug() {
  local serial="$1"
  echo "----- android runtime debug: ${serial} -----" >&2
  run_android_test "${serial}" report_runtime_debug_snapshot | tail -n 30 >&2 || true
  echo "----- android persisted debug: ${serial} -----" >&2
  run_android_test "${serial}" report_persisted_protocol_snapshot | tail -n 25 >&2 || true
}

report_ios_debug() {
  local udid="$1"
  local run_id="$2"
  echo "----- ios runtime debug: ${run_id} (${udid}) -----" >&2
  run_ios_test "${udid}" "${run_id}" report_runtime_debug_snapshot | tail -n 30 >&2 || true
  echo "----- ios persisted debug: ${run_id} (${udid}) -----" >&2
  run_ios_test "${udid}" "${run_id}" report_persisted_protocol_snapshot | tail -n 25 >&2 || true
}

cleanup() {
  local exit_code=$?
  if [[ ${exit_code} -ne 0 ]]; then
    echo "Mixed-platform matrix failed with exit code ${exit_code}. Dumping device snapshots." >&2
    [[ -n "${ANDROID_ADMIN_SERIAL}" ]] && report_android_debug "${ANDROID_ADMIN_SERIAL}"
    [[ -n "${ANDROID_MEMBER_SERIAL}" ]] && report_android_debug "${ANDROID_MEMBER_SERIAL}"
    [[ -n "${IOS_PRIMARY_UDID}" ]] && report_ios_debug "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}"
    [[ -n "${IOS_MEMBER_UDID}" ]] && report_ios_debug "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}"
    echo "Relay log: ${RELAY_LOG}" >&2
  fi
  if [[ -n "${RELAY_PID:-}" ]]; then
    stop_local_rust_relay "${RELAY_PID}"
  fi
  exit "${exit_code}"
}
report_error() {
  local exit_code=$?
  echo "Mixed-platform matrix failed near line ${BASH_LINENO[0]} while running: ${BASH_COMMAND}" >&2
  return "${exit_code}"
}
trap report_error ERR
trap cleanup EXIT

if [[ -z "${ANDROID_ADMIN_SERIAL}" || -z "${ANDROID_MEMBER_SERIAL}" ]]; then
  android_boot_avds=()
  admin_boot_index=-1
  member_boot_index=-1
  if [[ -z "${ANDROID_ADMIN_SERIAL}" ]]; then
    admin_boot_index=${#android_boot_avds[@]}
    android_boot_avds+=("${ANDROID_ADMIN_AVD}")
  fi
  if [[ -z "${ANDROID_MEMBER_SERIAL}" ]]; then
    member_boot_index=${#android_boot_avds[@]}
    android_boot_avds+=("${ANDROID_MEMBER_AVD}")
  fi
  android_boot_output="$("${ROOT_DIR}/scripts/run_android_emulators.sh" --headless "${android_boot_avds[@]}")"
  old_ifs="${IFS}"
  IFS=$'\n'
  android_boot=(${android_boot_output})
  IFS="${old_ifs}"
  [[ ${admin_boot_index} -ge 0 ]] && ANDROID_ADMIN_SERIAL="$(printf '%s\n' "${android_boot[${admin_boot_index}]}" | awk '{print $2}')"
  [[ ${member_boot_index} -ge 0 ]] && ANDROID_MEMBER_SERIAL="$(printf '%s\n' "${android_boot[${member_boot_index}]}" | awk '{print $2}')"
fi

for serial in "${ANDROID_ADMIN_SERIAL}" "${ANDROID_MEMBER_SERIAL}"; do
  if ! "${ADB}" -s "${serial}" get-state >/dev/null 2>&1; then
    echo "Android device ${serial} is not online." >&2
    exit 1
  fi
done

if [[ -z "${IOS_PRIMARY_UDID}" || -z "${IOS_MEMBER_UDID}" ]]; then
  ios_boot_output="$("${ROOT_DIR}/scripts/run_ios_simulators.sh" --no-open "${IOS_PRIMARY_SIM}" "${IOS_MEMBER_SIM}")"
  old_ifs="${IFS}"
  IFS=$'\n'
  ios_boot=(${ios_boot_output})
  IFS="${old_ifs}"
  [[ -z "${IOS_PRIMARY_UDID}" ]] && IOS_PRIMARY_UDID="$(extract_ios_udid "${ios_boot[0]}")"
  [[ -z "${IOS_MEMBER_UDID}" ]] && IOS_MEMBER_UDID="$(extract_ios_udid "${ios_boot[1]}")"
fi

require_value "ios_primary_udid" "${IOS_PRIMARY_UDID}"
require_value "ios_member_udid" "${IOS_MEMBER_UDID}"

if [[ "${USE_EXISTING_RELAY}" -eq 1 ]]; then
  assert_local_relay_healthy
  RELAY_PID=""
else
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
  assert_local_relay_healthy
fi

for serial in "${ANDROID_ADMIN_SERIAL}" "${ANDROID_MEMBER_SERIAL}"; do
  "${ADB}" -s "${serial}" reverse "tcp:$(local_relay_port)" "tcp:$(local_relay_port)" >/dev/null || true
done

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
  echo "Building Android debug apps against $(local_android_loopback_relay_url) ($(local_relay_set_id))"
  (
    cd "${ROOT_DIR}/android" &&
      IRIS_DEBUG_RELAYS="$(local_android_loopback_relay_url)" \
      IRIS_DEBUG_RELAY_SET_ID="$(local_relay_set_id)" \
      ./gradlew :app:installDebug :app:installDebugAndroidTest
  )

  echo "Building iOS XCFramework against $(local_ios_relay_url) ($(local_relay_set_id))"
  (
    cd "${ROOT_DIR}" &&
      IRIS_DEFAULT_RELAYS="$(local_ios_relay_url)" \
      IRIS_RELAY_SET_ID="$(local_relay_set_id)" \
      IRIS_TRUSTED_TEST_BUILD=true \
      ./scripts/ios-build ios-xcframework
  )
else
  echo "Skipping Android/iOS build; reusing installed artifacts"
fi

if [[ "${CLEAR_STATE}" -eq 1 ]]; then
  for serial in "${ANDROID_ADMIN_SERIAL}" "${ANDROID_MEMBER_SERIAL}"; do
    echo "Clearing Android app state on ${serial}"
    "${ADB}" -s "${serial}" shell pm clear "${ANDROID_APP_PACKAGE}" >/dev/null
    "${ADB}" -s "${serial}" shell pm clear "${ANDROID_TEST_PACKAGE}" >/dev/null || true
  done
fi

if [[ "${PHASE}" == "ios-owned" ]]; then
  echo "Loading mixed-platform state from ${STATE_FILE}"
  load_state
else
  echo "Creating Android and iOS identities"
  ANDROID_ADMIN_IDENTITY="$(run_android_test "${ANDROID_ADMIN_SERIAL}" create_account_and_report_identity)"
  ANDROID_ADMIN_NPUB="$(printf '%s\n' "${ANDROID_ADMIN_IDENTITY}" | extract_status npub)"
  ANDROID_ADMIN_HEX="$(printf '%s\n' "${ANDROID_ADMIN_IDENTITY}" | extract_status public_key_hex)"
  require_value ANDROID_ADMIN_NPUB "${ANDROID_ADMIN_NPUB}"
  require_value ANDROID_ADMIN_HEX "${ANDROID_ADMIN_HEX}"

  ANDROID_MEMBER_IDENTITY="$(run_android_test "${ANDROID_MEMBER_SERIAL}" create_account_and_report_identity)"
  ANDROID_MEMBER_NPUB="$(printf '%s\n' "${ANDROID_MEMBER_IDENTITY}" | extract_status npub)"
  ANDROID_MEMBER_HEX="$(printf '%s\n' "${ANDROID_MEMBER_IDENTITY}" | extract_status public_key_hex)"
  require_value ANDROID_MEMBER_NPUB "${ANDROID_MEMBER_NPUB}"
  require_value ANDROID_MEMBER_HEX "${ANDROID_MEMBER_HEX}"

  IOS_PRIMARY_IDENTITY="$(run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_account_and_report_identity)"
  IOS_PRIMARY_NPUB="$(printf '%s\n' "${IOS_PRIMARY_IDENTITY}" | extract_status npub)"
  IOS_PRIMARY_HEX="$(printf '%s\n' "${IOS_PRIMARY_IDENTITY}" | extract_status public_key_hex)"
  require_value IOS_PRIMARY_NPUB "${IOS_PRIMARY_NPUB}"
  require_value IOS_PRIMARY_HEX "${IOS_PRIMARY_HEX}"

  IOS_MEMBER_IDENTITY="$(run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_account_and_report_identity)"
  IOS_MEMBER_NPUB="$(printf '%s\n' "${IOS_MEMBER_IDENTITY}" | extract_status npub)"
  IOS_MEMBER_HEX="$(printf '%s\n' "${IOS_MEMBER_IDENTITY}" | extract_status public_key_hex)"
  require_value IOS_MEMBER_NPUB "${IOS_MEMBER_NPUB}"
  require_value IOS_MEMBER_HEX "${IOS_MEMBER_HEX}"
  write_state

  echo "Stabilizing Android creator -> iOS member and Android member transport"
  run_android_test "${ANDROID_ADMIN_SERIAL}" create_chat_from_args peer_input "${IOS_PRIMARY_NPUB}" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" create_chat_from_args peer_input "${ANDROID_MEMBER_NPUB}" >/dev/null
  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_chat_from_args peer_input "${ANDROID_ADMIN_NPUB}" >/dev/null
  run_android_test "${ANDROID_MEMBER_SERIAL}" create_chat_from_args peer_input "${ANDROID_ADMIN_NPUB}" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_peer_transport_ready_from_args peer_input "${IOS_PRIMARY_NPUB}" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_peer_transport_ready_from_args peer_input "${ANDROID_MEMBER_NPUB}" >/dev/null

  echo "Seeding direct chat for Android-created group"
  run_android_test "${ANDROID_ADMIN_SERIAL}" send_message_from_args peer_input "${IOS_PRIMARY_NPUB}" message "seed_android_to_ios" >/dev/null
  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args peer_input "${ANDROID_ADMIN_NPUB}" message "seed_android_to_ios" direction "incoming" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" send_message_from_args peer_input "${ANDROID_MEMBER_NPUB}" message "seed_android_to_android" >/dev/null
  run_android_test "${ANDROID_MEMBER_SERIAL}" wait_for_message_from_args peer_input "${ANDROID_ADMIN_NPUB}" message "seed_android_to_android" direction "incoming" >/dev/null

  echo "Creating Android-owned mixed group"
  ANDROID_GROUP_CREATE="$(run_android_test "${ANDROID_ADMIN_SERIAL}" create_group_from_args \
    group_name "${ANDROID_CREATOR_GROUP_NAME}" \
    member_inputs "${IOS_PRIMARY_NPUB},${ANDROID_MEMBER_NPUB}")"
  ANDROID_GROUP_CHAT_ID="$(printf '%s\n' "${ANDROID_GROUP_CREATE}" | extract_status chat_id)"
  require_value ANDROID_GROUP_CHAT_ID "${ANDROID_GROUP_CHAT_ID}"

  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_group_chat_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" >/dev/null
  run_android_test "${ANDROID_MEMBER_SERIAL}" wait_for_group_chat_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" >/dev/null

  echo "Exchanging messages in Android-owned mixed group"
  run_android_test "${ANDROID_ADMIN_SERIAL}" send_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_admin" >/dev/null
  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_admin" direction "incoming" >/dev/null
  run_android_test "${ANDROID_MEMBER_SERIAL}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_admin" direction "incoming" >/dev/null

  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" send_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_ios_member" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_ios_member" direction "incoming" >/dev/null
  run_android_test "${ANDROID_MEMBER_SERIAL}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_ios_member" direction "incoming" >/dev/null

  run_android_test "${ANDROID_MEMBER_SERIAL}" send_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_android_member" >/dev/null
  run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_android_member" direction "incoming" >/dev/null
  run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${ANDROID_GROUP_CHAT_ID}" message "android_creator_group_android_member" direction "incoming" >/dev/null
  write_state

  if [[ "${PHASE}" == "android-owned" ]]; then
    echo "Android-owned mixed group phase passed"
    echo "state_file=${STATE_FILE}"
    echo "relay_log=${RELAY_LOG}"
    echo "android_admin_serial=${ANDROID_ADMIN_SERIAL}"
    echo "android_member_serial=${ANDROID_MEMBER_SERIAL}"
    echo "ios_primary_udid=${IOS_PRIMARY_UDID}"
    echo "ios_member_udid=${IOS_MEMBER_UDID}"
    echo "android_group_chat_id=${ANDROID_GROUP_CHAT_ID}"
    exit 0
  fi
fi

echo "Stabilizing iOS creator -> Android member and iOS member transport"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_chat_from_args peer_input "${ANDROID_ADMIN_NPUB}" >/dev/null
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_chat_from_args peer_input "${IOS_PRIMARY_NPUB}" >/dev/null
run_android_test "${ANDROID_ADMIN_SERIAL}" create_chat_from_args peer_input "${IOS_MEMBER_NPUB}" >/dev/null
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" create_chat_from_args peer_input "${IOS_MEMBER_NPUB}" >/dev/null

echo "Seeding direct chat for iOS-created group"
echo "Sending iOS-created seed to Android member"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" send_message_from_args peer_input "${ANDROID_ADMIN_NPUB}" message "seed_ios_to_android" >/dev/null
echo "Waiting for Android member seed from iOS creator"
run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_message_from_args peer_input "${IOS_MEMBER_NPUB}" message "seed_ios_to_android" direction "incoming" >/dev/null
echo "Sending iOS-created seed to iOS member"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" send_message_from_args peer_input "${IOS_PRIMARY_NPUB}" message "seed_ios_to_ios" >/dev/null
echo "Waiting for iOS member seed from iOS creator"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args peer_input "${IOS_MEMBER_NPUB}" message "seed_ios_to_ios" direction "incoming" >/dev/null

echo "Creating iOS-owned mixed group"
IOS_GROUP_CREATE="$(run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" create_group_from_args \
  group_name "${IOS_CREATOR_GROUP_NAME}" \
  member_inputs "${ANDROID_ADMIN_NPUB},${IOS_PRIMARY_NPUB}")"
IOS_GROUP_CHAT_ID="$(printf '%s\n' "${IOS_GROUP_CREATE}" | extract_status chat_id)"
require_value IOS_GROUP_CHAT_ID "${IOS_GROUP_CHAT_ID}"
write_state

run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_group_chat_from_args chat_id "${IOS_GROUP_CHAT_ID}" >/dev/null
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_group_chat_from_args chat_id "${IOS_GROUP_CHAT_ID}" >/dev/null

echo "Exchanging messages in iOS-owned mixed group"
echo "Sending iOS-owned group message from iOS creator"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" send_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_admin" >/dev/null
echo "Waiting for Android member to receive iOS creator group message"
run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_admin" direction "incoming" >/dev/null
echo "Waiting for iOS member to receive iOS creator group message"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_admin" direction "incoming" >/dev/null

echo "Sending iOS-owned group message from Android member"
run_android_test "${ANDROID_ADMIN_SERIAL}" send_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_android_member" >/dev/null
echo "Waiting for iOS creator to receive Android member group message"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_android_member" direction "incoming" >/dev/null
echo "Waiting for iOS member to receive Android member group message"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_android_member" direction "incoming" >/dev/null

echo "Sending iOS-owned group message from iOS member"
run_ios_test "${IOS_PRIMARY_UDID}" "${IOS_PRIMARY_RUN_ID}" send_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_ios_member" >/dev/null
echo "Waiting for iOS creator to receive iOS member group message"
run_ios_test "${IOS_MEMBER_UDID}" "${IOS_MEMBER_RUN_ID}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_ios_member" direction "incoming" >/dev/null
echo "Waiting for Android member to receive iOS member group message"
run_android_test "${ANDROID_ADMIN_SERIAL}" wait_for_message_from_args chat_id "${IOS_GROUP_CHAT_ID}" message "ios_creator_group_ios_member" direction "incoming" >/dev/null

echo "Mixed-platform group chat matrix passed"
echo "relay_log=${RELAY_LOG}"
echo "android_admin_serial=${ANDROID_ADMIN_SERIAL}"
echo "android_member_serial=${ANDROID_MEMBER_SERIAL}"
echo "ios_primary_udid=${IOS_PRIMARY_UDID}"
echo "ios_member_udid=${IOS_MEMBER_UDID}"
echo "android_group_chat_id=${ANDROID_GROUP_CHAT_ID}"
echo "ios_group_chat_id=${IOS_GROUP_CHAT_ID}"
