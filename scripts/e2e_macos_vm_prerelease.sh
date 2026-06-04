#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"

RUN_TMPDIR="${TMPDIR:-/tmp}"
RUN_TMPDIR="${RUN_TMPDIR%/}"
RUN_DIR="${IRIS_MACOS_VM_E2E_RUN_DIR:-${RUN_TMPDIR}/iris-macos-vm-prerelease-$(iris_e2e_stamp)}"
RELAYS="${IRIS_E2E_RELAYS:-$(iris_e2e_default_public_relays)}"
TIMEOUT_SECS="${IRIS_MACOS_VM_E2E_TIMEOUT_SECS:-180}"
RUN_GUI=1
RUN_PUBLIC=1
RUN_MESH=1
REBUILD_HARNESS=1

usage() {
  cat <<'EOF'
usage: scripts/e2e_macos_vm_prerelease.sh [options]

Runs the macOS VM prerelease E2E gate. It is intended to run inside the
macos-utm VM, where it can exercise the macOS GUI app against public message
servers without touching connected phones on the host.

Options:
  --run-dir DIR          Artifact directory. Default: /tmp/iris-macos-vm-prerelease-<stamp>.
  --relays LIST          Public message servers, comma or | separated.
  --timeout-secs N       Harness wait timeout. Default: 180.
  --skip-gui             Skip the macOS GUI UI test suite.
  --skip-public          Skip the public relay restart/restore/group journey.
  --skip-mesh            Skip the four-device same-process mesh.
  --skip-build           Reuse existing harness build.
  --rebuild              Build harness before the public journey and mesh. Default.
  -h, --help             Show this help.

Outputs:
  manifest.env, steps.tsv, macos-vm-prerelease.log, result.env
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run-dir)
      RUN_DIR="$2"
      shift 2
      ;;
    --relays)
      RELAYS="$2"
      shift 2
      ;;
    --timeout-secs)
      TIMEOUT_SECS="$2"
      shift 2
      ;;
    --skip-gui)
      RUN_GUI=0
      shift
      ;;
    --skip-public)
      RUN_PUBLIC=0
      shift
      ;;
    --skip-mesh)
      RUN_MESH=0
      shift
      ;;
    --skip-build)
      REBUILD_HARNESS=0
      shift
      ;;
    --rebuild)
      REBUILD_HARNESS=1
      shift
      ;;
    -h|--help)
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

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS VM prerelease E2E requires macOS." >&2
  exit 1
fi

if [[ ! "${TIMEOUT_SECS}" =~ ^[0-9]+$ || "${TIMEOUT_SECS}" -lt 1 ]]; then
  echo "--timeout-secs must be a positive integer." >&2
  exit 2
fi

mkdir -p "${RUN_DIR}/logs" "${RUN_DIR}/harness-data"
LOG_FILE="${RUN_DIR}/macos-vm-prerelease.log"
STEPS_FILE="${RUN_DIR}/steps.tsv"
printf 'step\tstatus\tname\tlog\n' >"${STEPS_FILE}"
iris_e2e_record_repo_trace "${ROOT_DIR}" "${RUN_DIR}" || true

{
  printf 'run_dir=%s\n' "${RUN_DIR}"
  printf 'relays=%s\n' "${RELAYS}"
  printf 'timeout_secs=%s\n' "${TIMEOUT_SECS}"
  printf 'run_gui=%s\n' "${RUN_GUI}"
  printf 'run_public=%s\n' "${RUN_PUBLIC}"
  printf 'run_mesh=%s\n' "${RUN_MESH}"
  printf 'rebuild_harness=%s\n' "${REBUILD_HARNESS}"
} >"${RUN_DIR}/manifest.env"

STEP_COUNT=0
LAST_STEP_LOG=""

print_command() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
}

slug() {
  printf '%s' "$1" |
    tr '[:upper:] ' '[:lower:]-' |
    tr -cs 'a-z0-9._-' '-' |
    sed -E 's/^-+//; s/-+$//'
}

run_step() {
  local name="$1"
  shift
  STEP_COUNT=$((STEP_COUNT + 1))
  local log="${RUN_DIR}/logs/$(printf '%02d' "${STEP_COUNT}")-$(slug "${name}").log"
  LAST_STEP_LOG="${log}"
  echo
  echo "=== ${name} ===" | tee -a "${LOG_FILE}"
  print_command "$@" | tee -a "${LOG_FILE}" "${log}" >&2
  if "$@" 2>&1 | tee -a "${LOG_FILE}" "${log}"; then
    printf '%s\t%s\t%s\t%s\n' "${STEP_COUNT}" "passed" "${name}" "${log}" >>"${STEPS_FILE}"
  else
    local status=$?
    printf '%s\t%s\t%s\t%s\n' "${STEP_COUNT}" "failed" "${name}" "${log}" >>"${STEPS_FILE}"
    return "${status}"
  fi
}

run_harness() {
  local run_id="$1"
  local action="$2"
  local reset="$3"
  local rebuild="$4"
  shift 4
  local cmd=(
    python3 "${ROOT_DIR}/scripts/run_macos_harness.py"
    --run-id "${run_id}"
    --action "${action}"
    --data-root "${RUN_DIR}/harness-data"
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
  if ! printf '%s\n' "${output}" | grep -q '^INSTRUMENTATION_CODE: -1$'; then
    echo "macOS harness ${action} failed for ${run_id}" >&2
    printf '%s\n' "${output}" >&2
    return 1
  fi
  printf '%s\n' "${output}"
}

run_harness_step() {
  local name="$1"
  local run_id="$2"
  local action="$3"
  local reset="$4"
  local rebuild="$5"
  shift 5
  run_step "${name}" run_harness "${run_id}" "${action}" "${reset}" "${rebuild}" "$@"
}

extract_from_last_step() {
  local key="$1"
  iris_e2e_extract_status "${key}" <"${LAST_STEP_LOG}"
}

set_relays_and_connect() {
  local label="$1"
  local run_id="$2"
  run_harness_step "${label} set public relays" "${run_id}" set_relays_from_args 0 0 \
    relay_urls "${RELAYS}"
  run_harness_step "${label} connect public relays" "${run_id}" wait_for_connected_relay 0 0 \
    timeout_secs "${TIMEOUT_SECS}"
}

send_direct() {
  local name="$1"
  local sender_run_id="$2"
  local peer_input="$3"
  local message="$4"
  run_harness_step "${name}" "${sender_run_id}" send_message_from_args 0 0 \
    peer_input "${peer_input}" \
    message "${message}" \
    timeout_secs "${TIMEOUT_SECS}" \
    wait_for_relay_drain true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"
}

wait_direct() {
  local name="$1"
  local receiver_run_id="$2"
  local peer_input="$3"
  local message="$4"
  local direction="$5"
  run_harness_step "${name}" "${receiver_run_id}" wait_for_message_from_args 0 0 \
    peer_input "${peer_input}" \
    message "${message}" \
    direction "${direction}" \
    timeout_secs "${TIMEOUT_SECS}" \
    expected_count 1
}

run_gui_suite() {
  run_step "macOS GUI UI suite" "${ROOT_DIR}/scripts/macos-build" macos-ui-test
}

run_public_journey() {
  local stamp
  stamp="$(iris_e2e_stamp)"
  local harness_rebuild="${REBUILD_HARNESS}"

  run_harness_step "Alice create profile" alice create_account_and_report_identity 1 "${harness_rebuild}" \
    display_name "Mac Alice ${stamp}"
  ALICE_NPUB="$(extract_from_last_step npub)"
  ALICE_HEX="$(extract_from_last_step public_key_hex)"
  iris_e2e_require_value alice_npub "${ALICE_NPUB}"
  iris_e2e_require_value alice_hex "${ALICE_HEX}"
  set_relays_and_connect Alice alice
  run_harness_step "Alice publish profile" alice update_profile_metadata_from_args 0 0 \
    display_name "Mac Alice ${stamp}" \
    wait_for_relay_drain true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"

  run_harness_step "Bob create profile" bob create_account_and_report_identity 1 0 \
    display_name "Mac Bob ${stamp}"
  BOB_NPUB="$(extract_from_last_step npub)"
  BOB_HEX="$(extract_from_last_step public_key_hex)"
  iris_e2e_require_value bob_npub "${BOB_NPUB}"
  iris_e2e_require_value bob_hex "${BOB_HEX}"
  set_relays_and_connect Bob bob
  run_harness_step "Bob publish profile" bob update_profile_metadata_from_args 0 0 \
    display_name "Mac Bob ${stamp}" \
    wait_for_relay_drain true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"

  run_harness_step "Charlie create profile" charlie create_account_and_report_identity 1 0 \
    display_name "Mac Charlie ${stamp}"
  CHARLIE_NPUB="$(extract_from_last_step npub)"
  CHARLIE_HEX="$(extract_from_last_step public_key_hex)"
  iris_e2e_require_value charlie_npub "${CHARLIE_NPUB}"
  iris_e2e_require_value charlie_hex "${CHARLIE_HEX}"
  set_relays_and_connect Charlie charlie

  run_harness_step "Alice export secret key" alice export_secret_key 0 0
  ALICE_NSEC="$(extract_from_last_step secret_key)"
  iris_e2e_require_value alice_secret_key "${ALICE_NSEC}"

  local alice_to_bob="mac-public-alice-to-bob-${stamp}"
  local bob_to_alice="mac-public-bob-to-alice-${stamp}"
  local restored_to_bob="mac-public-restored-alice-to-bob-${stamp}"
  local group_from_alice="mac-public-group-alice-${stamp}"
  local group_from_bob="mac-public-group-bob-${stamp}"

  send_direct "Alice sends while Bob is offline" alice "${BOB_NPUB}" "${alice_to_bob}"
  wait_direct "Bob restarts and receives Alice message" bob "${ALICE_NPUB}" "${alice_to_bob}" incoming

  send_direct "Bob replies while Alice is offline" bob "${ALICE_NPUB}" "${bob_to_alice}"
  wait_direct "Alice restarts and receives Bob reply" alice "${BOB_NPUB}" "${bob_to_alice}" incoming

  run_harness_step "Alice restores from secret key" alice-restored restore_session_from_args 1 0 \
    secret_key "${ALICE_NSEC}" \
    expected_public_key_hex "${ALICE_HEX}" \
    wait_for_relay_drain true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"
  RESTORED_ALICE_HEX="$(extract_from_last_step public_key_hex)"
  iris_e2e_require_value restored_alice_hex "${RESTORED_ALICE_HEX}"
  set_relays_and_connect "Restored Alice" alice-restored
  send_direct "Restored Alice sends to Bob" alice-restored "${BOB_NPUB}" "${restored_to_bob}"
  wait_direct "Bob receives restored Alice message" bob "${ALICE_NPUB}" "${restored_to_bob}" incoming

  local group_name="Mac Public ${stamp}"
  run_harness_step "Alice creates three-person group" alice create_group_from_args 0 0 \
    group_name "${group_name}" \
    member_inputs "${BOB_NPUB},${CHARLIE_NPUB}" \
    wait_for_relay_drain true \
    relay_drain_runtime_only true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"
  GROUP_CHAT_ID="$(extract_from_last_step chat_id)"
  GROUP_ID="$(extract_from_last_step group_id)"
  iris_e2e_require_value group_chat_id "${GROUP_CHAT_ID}"
  iris_e2e_require_value group_id "${GROUP_ID}"

  run_harness_step "Bob restarts into group" bob wait_for_group_chat_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    timeout_secs "${TIMEOUT_SECS}"
  run_harness_step "Charlie restarts into group" charlie wait_for_group_chat_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    timeout_secs "${TIMEOUT_SECS}"

  run_harness_step "Alice sends group message" alice send_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_alice}" \
    timeout_secs "${TIMEOUT_SECS}" \
    wait_for_relay_drain true \
    relay_drain_runtime_only true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"
  run_harness_step "Bob receives Alice group message" bob wait_for_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_alice}" \
    direction incoming \
    timeout_secs "${TIMEOUT_SECS}" \
    expected_count 1
  run_harness_step "Charlie receives Alice group message" charlie wait_for_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_alice}" \
    direction incoming \
    timeout_secs "${TIMEOUT_SECS}" \
    expected_count 1

  run_harness_step "Bob sends group message" bob send_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_bob}" \
    timeout_secs "${TIMEOUT_SECS}" \
    wait_for_relay_drain true \
    relay_drain_runtime_only true \
    relay_drain_timeout_secs "${TIMEOUT_SECS}"
  run_harness_step "Alice receives Bob group message" alice wait_for_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_bob}" \
    direction incoming \
    timeout_secs "${TIMEOUT_SECS}" \
    expected_count 1
  run_harness_step "Charlie receives Bob group message" charlie wait_for_message_from_args 0 0 \
    chat_id "${GROUP_CHAT_ID}" \
    message "${group_from_bob}" \
    direction incoming \
    timeout_secs "${TIMEOUT_SECS}" \
    expected_count 1

  {
    printf 'alice_npub=%q\n' "${ALICE_NPUB}"
    printf 'alice_hex=%q\n' "${ALICE_HEX}"
    printf 'bob_npub=%q\n' "${BOB_NPUB}"
    printf 'bob_hex=%q\n' "${BOB_HEX}"
    printf 'charlie_npub=%q\n' "${CHARLIE_NPUB}"
    printf 'charlie_hex=%q\n' "${CHARLIE_HEX}"
    printf 'group_chat_id=%q\n' "${GROUP_CHAT_ID}"
    printf 'group_id=%q\n' "${GROUP_ID}"
    printf 'alice_to_bob=%q\n' "${alice_to_bob}"
    printf 'bob_to_alice=%q\n' "${bob_to_alice}"
    printf 'restored_to_bob=%q\n' "${restored_to_bob}"
    printf 'group_from_alice=%q\n' "${group_from_alice}"
    printf 'group_from_bob=%q\n' "${group_from_bob}"
  } >"${RUN_DIR}/public-result.env"
}

run_mesh() {
  run_harness_step "macOS four-device same-process mesh" macos-mesh same_process_multi_device_mesh 1 "${REBUILD_HARNESS}" \
    relay_urls "${RELAYS}" \
    timeout_secs "${TIMEOUT_SECS}"
}

if [[ "${RUN_GUI}" -eq 1 ]]; then
  run_gui_suite
fi

if [[ "${RUN_PUBLIC}" -eq 1 ]]; then
  run_public_journey
fi

if [[ "${RUN_MESH}" -eq 1 ]]; then
  run_mesh
fi

{
  printf 'run_dir=%q\n' "${RUN_DIR}"
  printf 'steps_file=%q\n' "${STEPS_FILE}"
  printf 'log_file=%q\n' "${LOG_FILE}"
  [[ -f "${RUN_DIR}/public-result.env" ]] && cat "${RUN_DIR}/public-result.env"
} >"${RUN_DIR}/result.env"

echo
echo "macOS VM prerelease E2E passed"
echo "run_dir=${RUN_DIR}"
