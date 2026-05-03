#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

SDK_DIR="$(iris_e2e_resolve_android_sdk "${ROOT_DIR}")"
ADB="${SDK_DIR}/platform-tools/adb"
ANDROID_HARNESS="${ROOT_DIR}/scripts/run_harness.py"
RUNNER="to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner"
CLASS="to.iris.chat.RealRelayHarnessTest"
APP_PACKAGE="to.iris.chat.debug"
TEST_PACKAGE="to.iris.chat.test"
AM_USER="${AM_USER:-0}"
RELAY_MODE="${IRIS_E2E_RELAY_MODE:-public}"
RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-prerelease-android-$(iris_e2e_stamp)}"
RELAYS="$(iris_e2e_default_public_relays)"
FRESH=0
SKIP_BUILD=0
HEADLESS=0

usage() {
  cat <<EOF
Usage: IRIS_ANDROID_E2E_AVDS="AliceAvd Alice2Avd BobAvd CharlieAvd" scripts/e2e_android_public_prerelease.sh [options]

Options:
  --fresh                 Clear app and test package state before running.
  --skip-build            Reuse installed Android artifacts.
  --headless              Launch missing emulators headlessly.
  --relay public|local    Relay mode. Default: ${RELAY_MODE}
  --relays CSV            Public relay CSV. Default: ${RELAYS}
  --relay-set-id ID       Relay set id. Default: generated per run.
  -h, --help              Show this help.

Environment:
  IRIS_ANDROID_E2E_AVDS     Required unless IRIS_ANDROID_E2E_SERIALS is set. Four unique AVD names.
  IRIS_ANDROID_E2E_SERIALS  Optional four adb serials, bypassing AVD launch.
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

case "${RELAY_MODE}" in
  public|local) ;;
  *)
    echo "Unknown relay mode: ${RELAY_MODE}" >&2
    exit 1
    ;;
esac

STAMP="$(iris_e2e_stamp)"
RUN_DIR="${IRIS_E2E_RUN_DIR:-/tmp/iris-e2e-android-${STAMP}}"
LOG_FILE="${RUN_DIR}/android-prerelease.log"
mkdir -p "${RUN_DIR}"
iris_e2e_record_repo_trace "${ROOT_DIR}" "${RUN_DIR}"

if [[ "${RELAY_MODE}" == "local" ]]; then
  RELAYS="$(local_android_relay_url)"
  RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-$(local_relay_set_id)}"
  RELAY_LOG="${RUN_DIR}/local-relay.log"
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
  echo "local_relay_pid=${RELAY_PID}" >>"${RUN_DIR}/repo-trace.env"
  echo "local_relay_log=${RELAY_LOG}" >>"${RUN_DIR}/repo-trace.env"
fi

read -r -a SERIALS <<<"${IRIS_ANDROID_E2E_SERIALS:-}"
if [[ ${#SERIALS[@]} -eq 0 ]]; then
  read -r -a AVDS <<<"${IRIS_ANDROID_E2E_AVDS:-}"
  if [[ ${#AVDS[@]} -lt 4 ]]; then
    echo "IRIS_ANDROID_E2E_AVDS must contain four unique AVD names." >&2
    echo "Available AVDs:" >&2
    "${ROOT_DIR}/scripts/run_android_emulators.sh" --list >&2 || true
    exit 1
  fi
  BOOT_CMD=("${ROOT_DIR}/scripts/run_android_emulators.sh")
  [[ "${HEADLESS}" -eq 1 ]] && BOOT_CMD+=(--headless)
  BOOT_CMD+=("${AVDS[@]:0:4}")
  SERIALS=()
  while IFS= read -r line; do
    SERIALS+=("$(printf '%s\n' "${line}" | awk '{print $2}')")
  done < <("${BOOT_CMD[@]}")
fi

if [[ ${#SERIALS[@]} -lt 4 ]]; then
  echo "Need four Android serials; got ${#SERIALS[@]}." >&2
  exit 1
fi
ALICE_SERIAL="${SERIALS[0]}"
ALICE_LINKED_SERIAL="${SERIALS[1]}"
BOB_SERIAL="${SERIALS[2]}"
CHARLIE_SERIAL="${SERIALS[3]}"

for serial in "${ALICE_SERIAL}" "${ALICE_LINKED_SERIAL}" "${BOB_SERIAL}" "${CHARLIE_SERIAL}"; do
  "${ADB}" -s "${serial}" get-state >/dev/null
done

{
  printf 'setup=android\n'
  printf 'relay_mode=%s\n' "${RELAY_MODE}"
  printf 'relay_set_id=%s\n' "${RELAY_SET_ID}"
  printf 'relays=%s\n' "${RELAYS}"
  printf 'alice_serial=%s\n' "${ALICE_SERIAL}"
  printf 'alice_linked_serial=%s\n' "${ALICE_LINKED_SERIAL}"
  printf 'bob_serial=%s\n' "${BOB_SERIAL}"
  printf 'charlie_serial=%s\n' "${CHARLIE_SERIAL}"
} >>"${RUN_DIR}/repo-trace.env"

run_android_test() {
  local serial="$1"
  local test_name="$2"
  shift 2
  local cmd=(
    python3
    "${ANDROID_HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${CLASS}"
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
    echo "Android harness ${test_name} did not report success on ${serial}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  printf '%s\n' "${output}"
}

start_android_test_background() {
  local serial="$1"
  local test_name="$2"
  local action_log="$3"
  local exit_file="$4"
  shift 4
  local cmd=(
    python3
    "${ANDROID_HARNESS}"
    --adb "${ADB}"
    --serial "${serial}"
    --runner "${RUNNER}"
    --class-name "${CLASS}"
    --test-name "${test_name}"
    --user "${AM_USER}"
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done

  (
    set +e
    {
      printf '+'
      printf ' %q' "${cmd[@]}"
      printf '\n'
    } | tee -a "${LOG_FILE}" "${action_log}" >&2
    "${cmd[@]}" 2>&1 | tee -a "${LOG_FILE}" "${action_log}"
    printf '%s\n' "${PIPESTATUS[0]}" >"${exit_file}"
  ) >/dev/null 2>&1 &
  IRIS_ANDROID_BACKGROUND_PID="$!"
}

report_android_debug() {
  local serial="$1"
  echo "----- Android runtime debug: ${serial} -----" | tee -a "${LOG_FILE}" >&2
  run_android_test "${serial}" report_runtime_debug_snapshot | tail -n 40 >&2 || true
  echo "----- Android persisted debug: ${serial} -----" | tee -a "${LOG_FILE}" >&2
  run_android_test "${serial}" report_persisted_protocol_snapshot | tail -n 30 >&2 || true
}

dump_debug_on_error() {
  local exit_code=$?
  echo "Android pre-release E2E failed with exit code ${exit_code}. Logs: ${RUN_DIR}" >&2
  report_android_debug "${ALICE_SERIAL}" || true
  report_android_debug "${ALICE_LINKED_SERIAL}" || true
  report_android_debug "${BOB_SERIAL}" || true
  report_android_debug "${CHARLIE_SERIAL}" || true
  exit "${exit_code}"
}
trap dump_debug_on_error ERR

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
  (
    cd "${ROOT_DIR}/android" &&
      IRIS_DEBUG_RELAYS="${RELAYS}" \
      IRIS_DEBUG_RELAY_SET_ID="${RELAY_SET_ID}" \
      ./gradlew :app:installDebug :app:installDebugAndroidTest
  ) 2>&1 | tee -a "${LOG_FILE}"
fi

if [[ "${FRESH}" -eq 1 ]]; then
  for serial in "${ALICE_SERIAL}" "${ALICE_LINKED_SERIAL}" "${BOB_SERIAL}" "${CHARLIE_SERIAL}"; do
    "${ADB}" -s "${serial}" shell pm clear "${APP_PACKAGE}" >/dev/null
    "${ADB}" -s "${serial}" shell pm clear "${TEST_PACKAGE}" >/dev/null || true
  done
fi

ALICE_IDENTITY="$(run_android_test "${ALICE_SERIAL}" create_account_and_report_identity \
  wait_for_relay_drain true relay_drain_timeout_secs 240)"
ALICE_NPUB="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status npub)"
ALICE_HEX="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value alice_npub "${ALICE_NPUB}"
iris_e2e_require_value alice_hex "${ALICE_HEX}"

BOB_IDENTITY="$(run_android_test "${BOB_SERIAL}" create_account_and_report_identity \
  wait_for_relay_drain true relay_drain_timeout_secs 240)"
BOB_NPUB="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status npub)"
BOB_HEX="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value bob_npub "${BOB_NPUB}"
iris_e2e_require_value bob_hex "${BOB_HEX}"

CHARLIE_IDENTITY="$(run_android_test "${CHARLIE_SERIAL}" create_account_and_report_identity \
  wait_for_relay_drain true relay_drain_timeout_secs 240)"
CHARLIE_NPUB="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status npub)"
CHARLIE_HEX="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value charlie_npub "${CHARLIE_NPUB}"
iris_e2e_require_value charlie_hex "${CHARLIE_HEX}"

LINK_LOG="${RUN_DIR}/alice-linked-authorization.log"
LINK_EXIT="${RUN_DIR}/alice-linked-authorization.exit"
IRIS_ANDROID_BACKGROUND_PID=""
start_android_test_background "${ALICE_LINKED_SERIAL}" start_link_invite_and_wait_for_authorization_from_args \
  "${LINK_LOG}" "${LINK_EXIT}" owner_input "${ALICE_NPUB}" authorization_state AUTHORIZED
LINK_PID="${IRIS_ANDROID_BACKGROUND_PID}"
iris_e2e_require_value link_pid "${LINK_PID}"
LINK_URL="$(iris_e2e_wait_for_status_in_file "${LINK_LOG}" invite_url 120)"
LINKED_DEVICE_NPUB="$(iris_e2e_wait_for_status_in_file "${LINK_LOG}" device_input 120)"
iris_e2e_require_value linked_device_npub "${LINKED_DEVICE_NPUB}"
iris_e2e_require_value link_url "${LINK_URL}"

run_android_test "${ALICE_SERIAL}" add_authorized_device_from_args \
  device_input "${LINK_URL}" wait_for_relay_drain true relay_drain_timeout_secs 240 >/dev/null
wait "${LINK_PID}"
LINK_STATUS="$(cat "${LINK_EXIT}")"
if [[ "${LINK_STATUS}" -ne 0 ]]; then
  echo "Linked-device authorization harness failed with exit code ${LINK_STATUS}" >&2
  exit "${LINK_STATUS}"
fi
if ! rg -q '^INSTRUMENTATION_CODE: -1$' "${LINK_LOG}"; then
  echo "Linked-device authorization harness did not report success" >&2
  exit 1
fi
ALICE_LINKED_AUTH="$(cat "${LINK_LOG}")"
ALICE_LINKED_OWNER_HEX="$(printf '%s\n' "${ALICE_LINKED_AUTH}" | iris_e2e_extract_status public_key_hex)"
ALICE_LINKED_HEX="$(printf '%s\n' "${ALICE_LINKED_AUTH}" | iris_e2e_extract_status device_public_key_hex)"
iris_e2e_require_value alice_linked_owner_hex "${ALICE_LINKED_OWNER_HEX}"
iris_e2e_require_value alice_linked_device_hex "${ALICE_LINKED_HEX}"
if [[ "${ALICE_LINKED_OWNER_HEX}" != "${ALICE_HEX}" ]]; then
  echo "Alice linked owner mismatch: expected ${ALICE_HEX}, got ${ALICE_LINKED_OWNER_HEX}" >&2
  exit 1
fi

ALICE_TO_BOB="android-public-alice-to-bob-${STAMP}"
ALICE_TO_CHARLIE="android-public-alice-to-charlie-${STAMP}"
run_android_test "${ALICE_SERIAL}" send_message_from_args \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" >/dev/null
run_android_test "${BOB_SERIAL}" wait_for_message_from_args \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_BOB}" direction incoming >/dev/null
run_android_test "${ALICE_LINKED_SERIAL}" wait_for_message_from_args \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" direction outgoing >/dev/null

run_android_test "${ALICE_SERIAL}" send_message_from_args \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" >/dev/null
run_android_test "${CHARLIE_SERIAL}" wait_for_message_from_args \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_CHARLIE}" direction incoming >/dev/null
run_android_test "${ALICE_LINKED_SERIAL}" wait_for_message_from_args \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" direction outgoing >/dev/null

GROUP_NAME="Alice-Bob-Charlie-${STAMP}"
GROUP_CREATE="$(run_android_test "${ALICE_SERIAL}" create_group_from_args \
  group_name "${GROUP_NAME}" member_inputs "${BOB_NPUB},${CHARLIE_NPUB}" \
  wait_for_relay_drain true relay_drain_runtime_only true relay_drain_timeout_secs 240)"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status chat_id)"
GROUP_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status group_id)"
iris_e2e_require_value group_chat_id "${GROUP_CHAT_ID}"
iris_e2e_require_value group_id "${GROUP_ID}"

run_android_test "${BOB_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_android_test "${CHARLIE_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null
run_android_test "${ALICE_LINKED_SERIAL}" wait_for_group_chat_from_args chat_id "${GROUP_CHAT_ID}" >/dev/null

RENAMED_GROUP_NAME="${GROUP_NAME}-renamed"
run_android_test "${ALICE_SERIAL}" update_group_name_from_args \
  group_id "${GROUP_ID}" group_name "${RENAMED_GROUP_NAME}" wait_for_relay_drain true relay_drain_runtime_only true relay_drain_timeout_secs 240 >/dev/null
run_android_test "${BOB_SERIAL}" wait_for_group_name_from_args \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_android_test "${CHARLIE_SERIAL}" wait_for_group_name_from_args \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_android_test "${ALICE_LINKED_SERIAL}" wait_for_group_name_from_args \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null

{
  printf 'alice_npub=%q\n' "${ALICE_NPUB}"
  printf 'alice_hex=%q\n' "${ALICE_HEX}"
  printf 'alice_linked_device_npub=%q\n' "${LINKED_DEVICE_NPUB}"
  printf 'alice_linked_device_hex=%q\n' "${ALICE_LINKED_HEX}"
  printf 'bob_npub=%q\n' "${BOB_NPUB}"
  printf 'bob_hex=%q\n' "${BOB_HEX}"
  printf 'charlie_npub=%q\n' "${CHARLIE_NPUB}"
  printf 'charlie_hex=%q\n' "${CHARLIE_HEX}"
  printf 'alice_to_bob_message=%q\n' "${ALICE_TO_BOB}"
  printf 'alice_to_charlie_message=%q\n' "${ALICE_TO_CHARLIE}"
  printf 'group_chat_id=%q\n' "${GROUP_CHAT_ID}"
  printf 'group_id=%q\n' "${GROUP_ID}"
  printf 'group_name=%q\n' "${RENAMED_GROUP_NAME}"
} >"${RUN_DIR}/result.env"

report_android_debug "${ALICE_SERIAL}"
report_android_debug "${ALICE_LINKED_SERIAL}"
report_android_debug "${BOB_SERIAL}"
report_android_debug "${CHARLIE_SERIAL}"

for serial in "${ALICE_SERIAL}" "${ALICE_LINKED_SERIAL}" "${BOB_SERIAL}" "${CHARLIE_SERIAL}"; do
  "${ADB}" -s "${serial}" shell am start --user "${AM_USER}" -n "${APP_PACKAGE}/.MainActivity" >/dev/null || true
done

trap - ERR
echo "Android pre-release E2E passed"
echo "run_dir=${RUN_DIR}"
echo "group_chat_id=${GROUP_CHAT_ID}"
echo "group_id=${GROUP_ID}"
