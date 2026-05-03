#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

SDK_DIR="$(iris_e2e_resolve_android_sdk "${ROOT_DIR}")"
ADB="${SDK_DIR}/platform-tools/adb"
ANDROID_HARNESS="${ROOT_DIR}/scripts/run_harness.py"
IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
ANDROID_RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CLASS="to.iris.chat.RealRelayHarnessTest"
ANDROID_APP_PACKAGE="to.iris.chat.debug"
ANDROID_TEST_PACKAGE="to.iris.chat.test"
IOS_BUNDLE_ID="${IOS_BUNDLE_ID:-to.iris.chat}"
AM_USER="${AM_USER:-0}"
IOS_A_UDID="${IRIS_MIXED_IOS_A_UDID:-5797FDF1-4808-4E9E-A2CB-7145A6766244}"
IOS_B_UDID="${IRIS_MIXED_IOS_B_UDID:-2844340E-3F62-4F48-8316-0DC582C12308}"
ALICE_PRIMARY_PLATFORM="${IRIS_MIXED_ALICE_PRIMARY:-ios}"
RELAY_MODE="${IRIS_E2E_RELAY_MODE:-public}"
RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-prerelease-mixed-$(iris_e2e_stamp)}"
RELAYS="$(iris_e2e_default_public_relays)"
FRESH=0
SKIP_BUILD=0
HEADLESS=0

usage() {
  cat <<EOF
Usage: IRIS_ANDROID_E2E_AVDS="AndroidA AndroidB" scripts/e2e_mixed_public_prerelease.sh [options]

Options:
  --fresh                     Clear app/test state before running.
  --skip-build                Reuse installed artifacts.
  --headless                  Launch missing Android emulators headlessly.
  --alice-primary ios|android Default: ${ALICE_PRIMARY_PLATFORM}
  --relay public|local        Relay mode. Default: ${RELAY_MODE}
  --relays CSV                Public relay CSV. Default: ${RELAYS}
  --relay-set-id ID           Relay set id. Default: generated per run.
  -h, --help                  Show this help.

Default topology:
  ios:     Alice primary, Charlie
  android: Alice linked, Bob

Flipped topology:
  android: Alice primary, Charlie
  ios:     Alice linked, Bob
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fresh)
      FRESH=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --headless)
      HEADLESS=1
      shift
      ;;
    --alice-primary)
      ALICE_PRIMARY_PLATFORM="$2"
      shift 2
      ;;
    --relay)
      RELAY_MODE="$2"
      shift 2
      ;;
    --relays)
      RELAYS="$2"
      shift 2
      ;;
    --relay-set-id)
      RELAY_SET_ID="$2"
      shift 2
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

case "${ALICE_PRIMARY_PLATFORM}" in
  ios|android) ;;
  *)
    echo "Unknown Alice primary platform: ${ALICE_PRIMARY_PLATFORM}" >&2
    exit 1
    ;;
esac
case "${RELAY_MODE}" in
  public|local) ;;
  *)
    echo "Unknown relay mode: ${RELAY_MODE}" >&2
    exit 1
    ;;
esac

STAMP="$(iris_e2e_stamp)"
RUN_DIR="${IRIS_E2E_RUN_DIR:-/tmp/iris-e2e-mixed-${STAMP}}"
LOG_FILE="${RUN_DIR}/mixed-prerelease.log"
mkdir -p "${RUN_DIR}/status"
iris_e2e_record_repo_trace "${ROOT_DIR}" "${RUN_DIR}"

if [[ "${RELAY_MODE}" == "local" ]]; then
  RELAYS="$(local_ios_relay_url)"
  ANDROID_RELAYS="$(local_android_relay_url)"
  RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-$(local_relay_set_id)}"
  RELAY_LOG="${RUN_DIR}/local-relay.log"
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
  echo "local_relay_pid=${RELAY_PID}" >>"${RUN_DIR}/repo-trace.env"
  echo "local_relay_log=${RELAY_LOG}" >>"${RUN_DIR}/repo-trace.env"
else
  ANDROID_RELAYS="${RELAYS}"
fi

read -r -a ANDROID_SERIALS <<<"${IRIS_ANDROID_E2E_SERIALS:-}"
if [[ ${#ANDROID_SERIALS[@]} -eq 0 ]]; then
  read -r -a AVDS <<<"${IRIS_ANDROID_E2E_AVDS:-}"
  if [[ ${#AVDS[@]} -lt 2 ]]; then
    echo "IRIS_ANDROID_E2E_AVDS must contain two unique AVD names for mixed E2E." >&2
    echo "Available AVDs:" >&2
    "${ROOT_DIR}/scripts/run_android_emulators.sh" --list >&2 || true
    exit 1
  fi
  BOOT_CMD=("${ROOT_DIR}/scripts/run_android_emulators.sh")
  [[ "${HEADLESS}" -eq 1 ]] && BOOT_CMD+=(--headless)
  BOOT_CMD+=("${AVDS[@]:0:2}")
  ANDROID_SERIALS=()
  while IFS= read -r line; do
    ANDROID_SERIALS+=("$(printf '%s\n' "${line}" | awk '{print $2}')")
  done < <("${BOOT_CMD[@]}")
fi
ANDROID_A_SERIAL="${ANDROID_SERIALS[0]}"
ANDROID_B_SERIAL="${ANDROID_SERIALS[1]}"

for udid in "${IOS_A_UDID}" "${IOS_B_UDID}"; do
  xcrun simctl boot "${udid}" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "${udid}" -b >/dev/null
done
open -a Simulator >/dev/null 2>&1 || true
for serial in "${ANDROID_A_SERIAL}" "${ANDROID_B_SERIAL}"; do
  "${ADB}" -s "${serial}" get-state >/dev/null
done

if [[ "${ALICE_PRIMARY_PLATFORM}" == "ios" ]]; then
  ALICE_PLATFORM="ios"; ALICE_ID="${IOS_A_UDID}"; ALICE_RUN_ID="alice"
  LINKED_PLATFORM="android"; LINKED_ID="${ANDROID_A_SERIAL}"; LINKED_RUN_ID="alice-linked"
  BOB_PLATFORM="android"; BOB_ID="${ANDROID_B_SERIAL}"; BOB_RUN_ID="bob"
  CHARLIE_PLATFORM="ios"; CHARLIE_ID="${IOS_B_UDID}"; CHARLIE_RUN_ID="charlie"
else
  ALICE_PLATFORM="android"; ALICE_ID="${ANDROID_A_SERIAL}"; ALICE_RUN_ID="alice"
  LINKED_PLATFORM="ios"; LINKED_ID="${IOS_A_UDID}"; LINKED_RUN_ID="alice-linked"
  BOB_PLATFORM="ios"; BOB_ID="${IOS_B_UDID}"; BOB_RUN_ID="bob"
  CHARLIE_PLATFORM="android"; CHARLIE_ID="${ANDROID_B_SERIAL}"; CHARLIE_RUN_ID="charlie"
fi

{
  printf 'setup=mixed\n'
  printf 'relay_mode=%s\n' "${RELAY_MODE}"
  printf 'relay_set_id=%s\n' "${RELAY_SET_ID}"
  printf 'ios_relays=%s\n' "${RELAYS}"
  printf 'android_relays=%s\n' "${ANDROID_RELAYS}"
  printf 'alice_platform=%s\n' "${ALICE_PLATFORM}"
  printf 'linked_platform=%s\n' "${LINKED_PLATFORM}"
  printf 'bob_platform=%s\n' "${BOB_PLATFORM}"
  printf 'charlie_platform=%s\n' "${CHARLIE_PLATFORM}"
} >>"${RUN_DIR}/repo-trace.env"

run_ios_test() {
  local udid="$1"
  local run_id="$2"
  local action="$3"
  local reset="$4"
  local rebuild="$5"
  shift 5
  local status_file="${RUN_DIR}/status/${run_id}-${action}.status"
  local cmd=(
    python3 "${IOS_HARNESS}"
    --udid "${udid}"
    --run-id "${run_id}"
    --action "${action}"
    --use-app-storage
    --data-root "${RUN_DIR}/harness-data"
    --arg "status_file=${status_file}"
  )
  [[ "${reset}" -eq 1 ]] && cmd+=(--reset)
  [[ "${rebuild}" -eq 1 ]] && cmd+=(--rebuild)
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  if ! output="$(iris_e2e_run_and_log "${LOG_FILE}" "${cmd[@]}")"; then
    printf '%s\n' "${output}" >&2
    return 1
  fi
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "iOS harness ${action} failed on ${udid}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  printf '%s\n' "${output}"
}

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
    --user "${AM_USER}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  if ! output="$(iris_e2e_run_and_log "${LOG_FILE}" "${cmd[@]}")"; then
    printf '%s\n' "${output}" >&2
    return 1
  fi
  if ! printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "Android harness ${test_name} failed on ${serial}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  printf '%s\n' "${output}"
}

run_device() {
  local platform="$1"
  local id="$2"
  local run_id="$3"
  local action="$4"
  local reset="$5"
  local rebuild="$6"
  shift 6
  if [[ "${platform}" == "ios" ]]; then
    run_ios_test "${id}" "${run_id}" "${action}" "${reset}" "${rebuild}" "$@"
  else
    run_android_test "${id}" "${action}" "$@"
  fi
}

report_device_debug() {
  local platform="$1"
  local id="$2"
  local run_id="$3"
  if [[ "${platform}" == "ios" ]]; then
    run_ios_test "${id}" "${run_id}" report_runtime_debug_snapshot 0 0 | tail -n 40 >&2 || true
    run_ios_test "${id}" "${run_id}" report_persisted_protocol_snapshot 0 0 | tail -n 30 >&2 || true
  else
    run_android_test "${id}" report_runtime_debug_snapshot | tail -n 40 >&2 || true
    run_android_test "${id}" report_persisted_protocol_snapshot | tail -n 30 >&2 || true
  fi
}

dump_debug_on_error() {
  local exit_code=$?
  echo "Mixed pre-release E2E failed with exit code ${exit_code}. Logs: ${RUN_DIR}" >&2
  report_device_debug "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}"
  report_device_debug "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}"
  report_device_debug "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}"
  report_device_debug "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}"
  exit "${exit_code}"
}
trap dump_debug_on_error ERR

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
  rm -rf "${ROOT_DIR}/ios/.build/harness-derived-data"
  (
    cd "${ROOT_DIR}" &&
      IRIS_DEFAULT_RELAYS="${RELAYS}" \
      IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
      IRIS_TRUSTED_TEST_BUILD=true \
      ./scripts/ios-build ios-xcframework
  ) 2>&1 | tee -a "${LOG_FILE}"
  (
    cd "${ROOT_DIR}/android" &&
      IRIS_DEBUG_RELAYS="${ANDROID_RELAYS}" \
      IRIS_DEBUG_RELAY_SET_ID="${RELAY_SET_ID}" \
      ./gradlew :app:installDebug :app:installDebugAndroidTest
  ) 2>&1 | tee -a "${LOG_FILE}"
fi

if [[ "${FRESH}" -eq 1 ]]; then
  for serial in "${ANDROID_A_SERIAL}" "${ANDROID_B_SERIAL}"; do
    "${ADB}" -s "${serial}" shell pm clear "${ANDROID_APP_PACKAGE}" >/dev/null
    "${ADB}" -s "${serial}" shell pm clear "${ANDROID_TEST_PACKAGE}" >/dev/null || true
  done
fi

ios_rebuild_for() {
  [[ "$1" == "ios" ]] && printf '1' || printf '0'
}

ALICE_IDENTITY="$(run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" create_account_and_report_identity "${FRESH}" "$(ios_rebuild_for "${ALICE_PLATFORM}")" \
  display_name Alice wait_for_relay_drain true relay_drain_timeout_secs 240)"
ALICE_NPUB="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status npub)"
ALICE_HEX="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value alice_npub "${ALICE_NPUB}"
iris_e2e_require_value alice_hex "${ALICE_HEX}"

BOB_IDENTITY="$(run_device "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}" create_account_and_report_identity "${FRESH}" "$(ios_rebuild_for "${BOB_PLATFORM}")" \
  display_name Bob wait_for_relay_drain true relay_drain_timeout_secs 240)"
BOB_NPUB="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status npub)"
BOB_HEX="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value bob_npub "${BOB_NPUB}"
iris_e2e_require_value bob_hex "${BOB_HEX}"

CHARLIE_IDENTITY="$(run_device "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}" create_account_and_report_identity "${FRESH}" "$(ios_rebuild_for "${CHARLIE_PLATFORM}")" \
  display_name Charlie wait_for_relay_drain true relay_drain_timeout_secs 240)"
CHARLIE_NPUB="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status npub)"
CHARLIE_HEX="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value charlie_npub "${CHARLIE_NPUB}"
iris_e2e_require_value charlie_hex "${CHARLIE_HEX}"

LINK_START="$(run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" start_linked_device_and_report_identity "${FRESH}" "$(ios_rebuild_for "${LINKED_PLATFORM}")" \
  owner_input "${ALICE_NPUB}")"
LINK_INPUT="$(printf '%s\n' "${LINK_START}" | iris_e2e_extract_status link_url)"
if [[ -z "${LINK_INPUT}" ]]; then
  LINK_INPUT="$(printf '%s\n' "${LINK_START}" | iris_e2e_extract_status device_npub)"
fi
if [[ -z "${LINK_INPUT}" ]]; then
  LINK_INPUT="$(printf '%s\n' "${LINK_START}" | iris_e2e_extract_status device_input)"
fi
iris_e2e_require_value link_input "${LINK_INPUT}"

run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" add_authorized_device_from_args 0 0 \
  device_input "${LINK_INPUT}" wait_for_relay_drain true relay_drain_timeout_secs 240 >/dev/null
AUTH_STATE="AUTHORIZED"
[[ "${LINKED_PLATFORM}" == "ios" ]] && AUTH_STATE="authorized"
run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" wait_for_authorization_state_from_args 0 0 \
  authorization_state "${AUTH_STATE}" >/dev/null

ALICE_TO_BOB="mixed-public-alice-to-bob-${STAMP}"
ALICE_TO_CHARLIE="mixed-public-alice-to-charlie-${STAMP}"
run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" send_message_from_args 0 0 \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" >/dev/null
run_device "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}" wait_for_message_from_args 0 0 \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_BOB}" direction incoming >/dev/null
run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" wait_for_message_from_args 0 0 \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" direction outgoing >/dev/null

run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" send_message_from_args 0 0 \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" >/dev/null
run_device "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}" wait_for_message_from_args 0 0 \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_CHARLIE}" direction incoming >/dev/null
run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" wait_for_message_from_args 0 0 \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" direction outgoing >/dev/null

GROUP_NAME="Mixed-Alice-Bob-Charlie-${STAMP}"
GROUP_CREATE="$(run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" create_group_from_args 0 0 \
  group_name "${GROUP_NAME}" member_inputs "${BOB_NPUB},${CHARLIE_NPUB}" \
  wait_for_relay_drain true relay_drain_runtime_only true relay_drain_timeout_secs 240)"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status chat_id)"
GROUP_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status group_id)"
iris_e2e_require_value group_chat_id "${GROUP_CHAT_ID}"
iris_e2e_require_value group_id "${GROUP_ID}"

run_device "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}" wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null
run_device "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}" wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null
run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null

RENAMED_GROUP_NAME="${GROUP_NAME}-renamed"
run_device "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}" update_group_name_from_args 0 0 \
  group_id "${GROUP_ID}" group_name "${RENAMED_GROUP_NAME}" wait_for_relay_drain true relay_drain_runtime_only true relay_drain_timeout_secs 240 >/dev/null
run_device "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}" wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_device "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}" wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_device "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}" wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null

{
  printf 'alice_platform=%q\n' "${ALICE_PLATFORM}"
  printf 'linked_platform=%q\n' "${LINKED_PLATFORM}"
  printf 'bob_platform=%q\n' "${BOB_PLATFORM}"
  printf 'charlie_platform=%q\n' "${CHARLIE_PLATFORM}"
  printf 'alice_npub=%q\n' "${ALICE_NPUB}"
  printf 'alice_hex=%q\n' "${ALICE_HEX}"
  printf 'bob_npub=%q\n' "${BOB_NPUB}"
  printf 'bob_hex=%q\n' "${BOB_HEX}"
  printf 'charlie_npub=%q\n' "${CHARLIE_NPUB}"
  printf 'charlie_hex=%q\n' "${CHARLIE_HEX}"
  printf 'link_input=%q\n' "${LINK_INPUT}"
  printf 'group_chat_id=%q\n' "${GROUP_CHAT_ID}"
  printf 'group_id=%q\n' "${GROUP_ID}"
  printf 'group_name=%q\n' "${RENAMED_GROUP_NAME}"
} >"${RUN_DIR}/result.env"

report_device_debug "${ALICE_PLATFORM}" "${ALICE_ID}" "${ALICE_RUN_ID}"
report_device_debug "${LINKED_PLATFORM}" "${LINKED_ID}" "${LINKED_RUN_ID}"
report_device_debug "${BOB_PLATFORM}" "${BOB_ID}" "${BOB_RUN_ID}"
report_device_debug "${CHARLIE_PLATFORM}" "${CHARLIE_ID}" "${CHARLIE_RUN_ID}"

for udid in "${IOS_A_UDID}" "${IOS_B_UDID}"; do
  xcrun simctl launch "${udid}" "${IOS_BUNDLE_ID}" >/dev/null 2>&1 || true
done
for serial in "${ANDROID_A_SERIAL}" "${ANDROID_B_SERIAL}"; do
  "${ADB}" -s "${serial}" shell am start --user "${AM_USER}" -n "${ANDROID_APP_PACKAGE}/.MainActivity" >/dev/null || true
done

trap - ERR
echo "Mixed pre-release E2E passed"
echo "run_dir=${RUN_DIR}"
echo "group_chat_id=${GROUP_CHAT_ID}"
echo "group_id=${GROUP_ID}"
