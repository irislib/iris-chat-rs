#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
IOS_BUNDLE_ID="${IOS_BUNDLE_ID:-to.iris.chat}"
ALICE_UDID="${IRIS_IOS_ALICE_UDID:-5797FDF1-4808-4E9E-A2CB-7145A6766244}"
ALICE_LINKED_UDID="${IRIS_IOS_ALICE_LINKED_UDID:-6EB8A247-5E53-4CC8-8C65-BEB613914390}"
BOB_UDID="${IRIS_IOS_BOB_UDID:-A30E29AB-441A-4FA2-9848-764382A6C5C5}"
CHARLIE_UDID="${IRIS_IOS_CHARLIE_UDID:-2844340E-3F62-4F48-8316-0DC582C12308}"
RELAY_MODE="${IRIS_E2E_RELAY_MODE:-public}"
RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-prerelease-ios-$(iris_e2e_stamp)}"
RELAYS="$(iris_e2e_default_public_relays)"
FRESH=0
SKIP_BUILD=0
KILL_LEFTOVERS=0

usage() {
  cat <<EOF
Usage: scripts/e2e_ios_public_prerelease.sh [options]

Options:
  --fresh                 Reset app storage/keychain on the first action per simulator.
  --skip-build            Reuse existing build artifacts.
  --relay public|local    Relay mode. Default: ${RELAY_MODE}
  --relays CSV            Public relay CSV. Default: ${RELAYS}
  --relay-set-id ID       Relay set id. Default: generated per run.
  --kill-leftovers        Stop stale harness/xcodebuild processes before starting.
  -h, --help              Show this help.

Environment:
  IRIS_IOS_ALICE_UDID, IRIS_IOS_ALICE_LINKED_UDID, IRIS_IOS_BOB_UDID,
  IRIS_IOS_CHARLIE_UDID, IRIS_E2E_RELAYS, IRIS_E2E_RELAY_SET_ID
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
    --kill-leftovers)
      KILL_LEFTOVERS=1
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

case "${RELAY_MODE}" in
  public|local) ;;
  *)
    echo "Unknown relay mode: ${RELAY_MODE}" >&2
    exit 1
    ;;
esac

STAMP="$(iris_e2e_stamp)"
RUN_DIR="${IRIS_E2E_RUN_DIR:-/tmp/iris-e2e-ios-${STAMP}}"
LOG_FILE="${RUN_DIR}/ios-prerelease.log"
mkdir -p "${RUN_DIR}/status"
iris_e2e_record_repo_trace "${ROOT_DIR}" "${RUN_DIR}"
IOS_E2E_UDIDS=("${ALICE_UDID}" "${ALICE_LINKED_UDID}" "${BOB_UDID}" "${CHARLIE_UDID}")

if [[ "${RELAY_MODE}" == "local" ]]; then
  RELAYS="$(local_ios_relay_url)"
  RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-$(local_relay_set_id)}"
  RELAY_LOG="${RUN_DIR}/local-relay.log"
  RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
  echo "local_relay_pid=${RELAY_PID}" >>"${RUN_DIR}/repo-trace.env"
  echo "local_relay_log=${RELAY_LOG}" >>"${RUN_DIR}/repo-trace.env"
fi

{
  printf 'setup=ios\n'
  printf 'relay_mode=%s\n' "${RELAY_MODE}"
  printf 'relay_set_id=%s\n' "${RELAY_SET_ID}"
  printf 'relays=%s\n' "${RELAYS}"
  printf 'alice_udid=%s\n' "${ALICE_UDID}"
  printf 'alice_linked_udid=%s\n' "${ALICE_LINKED_UDID}"
  printf 'bob_udid=%s\n' "${BOB_UDID}"
  printf 'charlie_udid=%s\n' "${CHARLIE_UDID}"
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
    python3
    "${IOS_HARNESS}"
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
    echo "iOS harness ${action} did not report success on ${udid}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  printf '%s\n' "${output}"
}

start_ios_test_background() {
  local udid="$1"
  local run_id="$2"
  local action="$3"
  local reset="$4"
  local rebuild="$5"
  local action_log="$6"
  local exit_file="$7"
  shift 7

  local status_file="${RUN_DIR}/status/${run_id}-${action}.status"
  local cmd=(
    python3
    "${IOS_HARNESS}"
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
  IRIS_IOS_BACKGROUND_PID="$!"
}

wait_for_status_in_file() {
  local file="$1"
  local key="$2"
  local timeout_secs="$3"
  local deadline=$((SECONDS + timeout_secs))
  local value=""
  while (( SECONDS < deadline )); do
    if [[ -f "${file}" ]]; then
      value="$(iris_e2e_extract_status "${key}" <"${file}")"
      if [[ -n "${value}" ]]; then
        printf '%s\n' "${value}"
        return 0
      fi
    fi
    sleep 1
  done
  echo "Timed out waiting for ${key} in ${file}" >&2
  return 1
}

report_ios_debug() {
  local udid="$1"
  local run_id="$2"
  echo "----- iOS runtime debug: ${run_id} (${udid}) -----" | tee -a "${LOG_FILE}" >&2
  run_ios_test "${udid}" "${run_id}" report_runtime_debug_snapshot 0 0 | tail -n 40 >&2 || true
  echo "----- iOS persisted debug: ${run_id} (${udid}) -----" | tee -a "${LOG_FILE}" >&2
  run_ios_test "${udid}" "${run_id}" report_persisted_protocol_snapshot 0 0 | tail -n 30 >&2 || true
}

dump_debug_on_error() {
  local exit_code=$?
  echo "iOS pre-release E2E failed with exit code ${exit_code}. Logs: ${RUN_DIR}" >&2
  report_ios_debug "${ALICE_UDID}" alice || true
  report_ios_debug "${ALICE_LINKED_UDID}" alice-linked || true
  report_ios_debug "${BOB_UDID}" bob || true
  report_ios_debug "${CHARLIE_UDID}" charlie || true
  exit "${exit_code}"
}
trap dump_debug_on_error ERR

cleanup_ios_simulators() {
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
    return 0
  fi
  iris_e2e_shutdown_ios_simulators "${IOS_E2E_UDIDS[@]}"
  if [[ "${IRIS_E2E_CLOSE_STALE_IOS_SIMS:-1}" != "0" ]]; then
    iris_e2e_shutdown_stale_ios_simulators
  fi
}
trap 'exit_code=$?; cleanup_ios_simulators; exit "${exit_code}"' EXIT

if [[ "${KILL_LEFTOVERS}" -eq 1 ]]; then
  pkill -f "run_ios_harness.py" >/dev/null 2>&1 || true
  pkill -f "InteropHarnessTests" >/dev/null 2>&1 || true
fi

if [[ "${IRIS_E2E_CLOSE_STALE_IOS_SIMS:-1}" != "0" ]]; then
  iris_e2e_shutdown_stale_ios_simulators "${IOS_E2E_UDIDS[@]}"
fi
for udid in "${ALICE_UDID}" "${ALICE_LINKED_UDID}" "${BOB_UDID}" "${CHARLIE_UDID}"; do
  xcrun simctl boot "${udid}" >/dev/null 2>&1 || true
  iris_e2e_wait_for_ios_bootstatus "${udid}"
done
if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
  open -a Simulator >/dev/null 2>&1 || true
fi

if [[ "${SKIP_BUILD}" -eq 0 ]]; then
  rm -rf "${ROOT_DIR}/ios/.build/harness-derived-data"
  (
    cd "${ROOT_DIR}" &&
      IRIS_DEFAULT_RELAYS="${RELAYS}" \
      IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
      IRIS_TRUSTED_TEST_BUILD=true \
      ./scripts/ios-build ios-xcframework
  ) 2>&1 | tee -a "${LOG_FILE}"
fi

RESET_ARG="${FRESH}"
ALICE_IDENTITY="$(run_ios_test "${ALICE_UDID}" alice create_account_and_report_identity "${RESET_ARG}" 1 \
  display_name Alice wait_for_relay_drain true relay_drain_timeout_secs 240)"
ALICE_NPUB="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status npub)"
ALICE_HEX="$(printf '%s\n' "${ALICE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value alice_npub "${ALICE_NPUB}"
iris_e2e_require_value alice_hex "${ALICE_HEX}"

BOB_IDENTITY="$(run_ios_test "${BOB_UDID}" bob create_account_and_report_identity "${RESET_ARG}" 0 \
  display_name Bob wait_for_relay_drain true relay_drain_timeout_secs 240)"
BOB_NPUB="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status npub)"
BOB_HEX="$(printf '%s\n' "${BOB_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value bob_npub "${BOB_NPUB}"
iris_e2e_require_value bob_hex "${BOB_HEX}"

CHARLIE_IDENTITY="$(run_ios_test "${CHARLIE_UDID}" charlie create_account_and_report_identity "${RESET_ARG}" 0 \
  display_name Charlie wait_for_relay_drain true relay_drain_timeout_secs 240)"
CHARLIE_NPUB="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status npub)"
CHARLIE_HEX="$(printf '%s\n' "${CHARLIE_IDENTITY}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value charlie_npub "${CHARLIE_NPUB}"
iris_e2e_require_value charlie_hex "${CHARLIE_HEX}"

LINK_LOG="${RUN_DIR}/alice-linked-authorization.log"
LINK_EXIT="${RUN_DIR}/alice-linked-authorization.exit"
LINK_STATUS_FILE="${RUN_DIR}/status/alice-linked-start_linked_device_wait_authorized_from_args.status"
IRIS_IOS_BACKGROUND_PID=""
start_ios_test_background "${ALICE_LINKED_UDID}" alice-linked start_linked_device_wait_authorized_from_args "${RESET_ARG}" 0 \
  "${LINK_LOG}" "${LINK_EXIT}" owner_input "${ALICE_NPUB}"
LINK_PID="${IRIS_IOS_BACKGROUND_PID}"
iris_e2e_require_value link_pid "${LINK_PID}"
LINK_URL="$(wait_for_status_in_file "${LINK_STATUS_FILE}" link_url 120)"
LINK_DEVICE_INPUT="$(wait_for_status_in_file "${LINK_STATUS_FILE}" device_input 120)"
iris_e2e_require_value link_url "${LINK_URL}"
iris_e2e_require_value link_device_input "${LINK_DEVICE_INPUT}"

run_ios_test "${ALICE_UDID}" alice add_authorized_device_from_args 0 0 \
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
ALICE_LINKED_AUTH="$(cat "${LINK_LOG}" "${LINK_STATUS_FILE}")"
ALICE_LINKED_HEX="$(printf '%s\n' "${ALICE_LINKED_AUTH}" | iris_e2e_extract_status public_key_hex)"
iris_e2e_require_value alice_linked_hex "${ALICE_LINKED_HEX}"
if [[ "${ALICE_LINKED_HEX}" != "${ALICE_HEX}" ]]; then
  echo "Alice linked owner mismatch: expected ${ALICE_HEX}, got ${ALICE_LINKED_HEX}" >&2
  exit 1
fi

ALICE_TO_BOB="ios-public-alice-to-bob-${STAMP}"
ALICE_TO_CHARLIE="ios-public-alice-to-charlie-${STAMP}"
run_ios_test "${ALICE_UDID}" alice send_message_from_args 0 0 \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" \
  wait_for_relay_drain true relay_drain_timeout_secs 240 >/dev/null
run_ios_test "${BOB_UDID}" bob wait_for_message_from_args 0 0 \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_BOB}" direction incoming >/dev/null
run_ios_test "${ALICE_LINKED_UDID}" alice-linked wait_for_message_from_args 0 0 \
  peer_input "${BOB_NPUB}" message "${ALICE_TO_BOB}" direction outgoing >/dev/null

run_ios_test "${ALICE_UDID}" alice send_message_from_args 0 0 \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" \
  wait_for_relay_drain true relay_drain_timeout_secs 240 >/dev/null
run_ios_test "${CHARLIE_UDID}" charlie wait_for_message_from_args 0 0 \
  peer_input "${ALICE_NPUB}" message "${ALICE_TO_CHARLIE}" direction incoming >/dev/null
run_ios_test "${ALICE_LINKED_UDID}" alice-linked wait_for_message_from_args 0 0 \
  peer_input "${CHARLIE_NPUB}" message "${ALICE_TO_CHARLIE}" direction outgoing >/dev/null

GROUP_NAME="Alice-Bob-Charlie-${STAMP}"
GROUP_CREATE="$(run_ios_test "${ALICE_UDID}" alice create_group_from_args 0 0 \
  group_name "${GROUP_NAME}" member_inputs "${BOB_NPUB},${CHARLIE_NPUB}" \
  wait_for_relay_drain true relay_drain_timeout_secs 240)"
GROUP_CHAT_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status chat_id)"
GROUP_ID="$(printf '%s\n' "${GROUP_CREATE}" | iris_e2e_extract_status group_id)"
iris_e2e_require_value group_chat_id "${GROUP_CHAT_ID}"
iris_e2e_require_value group_id "${GROUP_ID}"

run_ios_test "${BOB_UDID}" bob wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null
run_ios_test "${CHARLIE_UDID}" charlie wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null
run_ios_test "${ALICE_LINKED_UDID}" alice-linked wait_for_group_chat_from_args 0 0 chat_id "${GROUP_CHAT_ID}" >/dev/null

RENAMED_GROUP_NAME="${GROUP_NAME}-renamed"
run_ios_test "${ALICE_UDID}" alice update_group_name_from_args 0 0 \
  group_id "${GROUP_ID}" group_name "${RENAMED_GROUP_NAME}" \
  wait_for_relay_drain true relay_drain_timeout_secs 240 >/dev/null
run_ios_test "${BOB_UDID}" bob wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_ios_test "${CHARLIE_UDID}" charlie wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null
run_ios_test "${ALICE_LINKED_UDID}" alice-linked wait_for_group_name_from_args 0 0 \
  chat_id "${GROUP_CHAT_ID}" group_name "${RENAMED_GROUP_NAME}" >/dev/null

{
  printf 'alice_npub=%q\n' "${ALICE_NPUB}"
  printf 'alice_hex=%q\n' "${ALICE_HEX}"
  printf 'alice_linked_device_input=%q\n' "${LINK_DEVICE_INPUT}"
  printf 'link_url=%q\n' "${LINK_URL}"
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

report_ios_debug "${ALICE_UDID}" alice
report_ios_debug "${ALICE_LINKED_UDID}" alice-linked
report_ios_debug "${BOB_UDID}" bob
report_ios_debug "${CHARLIE_UDID}" charlie

if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
  for udid in "${ALICE_UDID}" "${ALICE_LINKED_UDID}" "${BOB_UDID}" "${CHARLIE_UDID}"; do
    xcrun simctl launch "${udid}" "${IOS_BUNDLE_ID}" >/dev/null 2>&1 || true
  done
fi

trap - ERR
echo "iOS pre-release E2E passed"
echo "run_dir=${RUN_DIR}"
echo "group_chat_id=${GROUP_CHAT_ID}"
echo "group_id=${GROUP_ID}"
