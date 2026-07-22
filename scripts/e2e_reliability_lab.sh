#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"

RUN_DIR="${IRIS_E2E_RELIABILITY_RUN_DIR:-${TMPDIR:-/tmp}/iris-reliability-lab-$(iris_e2e_stamp)}"
RELAY="${IRIS_E2E_RELIABILITY_RELAY:-local}"
IOS_SIMULATORS="${IRIS_IOS_E2E_SIMULATORS:-Iris Chat iPhone|Iris Chat iPhone 2}"
ANDROID_AVDS="${IRIS_ANDROID_E2E_AVDS:-}"
ANDROID_SERIALS="${IRIS_ANDROID_E2E_SERIALS:-}"
SOAK_ITERATIONS="${IRIS_E2E_SOAK_ITERATIONS:-3}"
ALLOW_PHYSICAL="${IRIS_E2E_ALLOW_PHYSICAL:-0}"
DRY_RUN=0
HEADLESS=0
WIPE_DATA=0
SKIP_BUILD=0
LIST_ONLY=0
STEP_COUNT=0
TIERS=()
ANDROID_AVD_LIST=()
ANDROID_SERIAL_LIST=()

usage() {
  cat <<'EOF'
Usage: scripts/e2e_reliability_lab.sh [options]

Runs a real-use messaging reliability lab from one command. By default it uses
iOS simulators and Android emulators only, even when phones are connected.

Options:
  --list                    List lab tiers and exit.
  --tier NAME               smoke, daily, soak, docker, release, or physical.
                            Can be repeated or comma-separated. Default: smoke.
  --relay local|public      Relay mode for flows that support both. Default: local.
  --run-dir DIR             Artifact directory. Default: /tmp/iris-reliability-lab-<stamp>.
  --ios-simulators LIST     Two simulator names separated by comma or |.
  --android-avds LIST       Android AVD names separated by comma, |, or spaces.
  --android-serials LIST    Physical adb serials for --tier physical.
  --soak-iterations N       Restart/message cycles for --tier soak. Default: 3.
  --headless                Launch Android emulators headlessly.
  --wipe-data               Wipe AVD data before launch.
  --skip-build              Reuse installed artifacts where supported.
  --allow-physical          Permit --tier physical to reset selected phones.
  --dry-run                 Print the commands without running them.
  -h, --help                Show this help.

Environment mirrors the long options:
  IRIS_IOS_E2E_SIMULATORS, IRIS_ANDROID_E2E_AVDS, IRIS_ANDROID_E2E_SERIALS,
  IRIS_E2E_SOAK_ITERATIONS, IRIS_E2E_ALLOW_PHYSICAL,
  IRIS_E2E_RELIABILITY_RELAY, IRIS_E2E_RELIABILITY_RUN_DIR.
EOF
}

list_tiers() {
  cat <<'EOF'
smoke    iOS secret-key restore, Android offline/restart recovery, mixed cold group invite
daily    smoke plus Android restore, mixed offline/restart, group membership, app parity, linked-device revocation, multi-device mesh
soak     repeated mixed restart/message cycles
docker   production message-server CLI Docker e2e
release  daily plus soak plus docker, still emulator/simulator-only
physical selected phone-backed flows; requires --allow-physical and --android-serials
EOF
}

append_tiers() {
  local raw="$1"
  local tier=""
  raw="${raw//,/ }"
  for tier in ${raw}; do
    [[ -n "${tier}" ]] || continue
    TIERS+=("${tier}")
  done
}

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

split_android_values() {
  local raw="$1"
  printf '%s\n' "${raw}" | tr ',|' '  ' | awk '{ for (i = 1; i <= NF; i++) print $i }'
}

append_unique_avd() {
  local avd="$1"
  local existing=""
  [[ -n "${avd}" ]] || return 0
  for existing in ${ANDROID_AVD_LIST[@]+"${ANDROID_AVD_LIST[@]}"}; do
    [[ "${existing}" != "${avd}" ]] || return 0
  done
  ANDROID_AVD_LIST+=("${avd}")
}

append_unique_serial() {
  local serial="$1"
  local existing=""
  [[ -n "${serial}" ]] || return 0
  for existing in ${ANDROID_SERIAL_LIST[@]+"${ANDROID_SERIAL_LIST[@]}"}; do
    [[ "${existing}" != "${serial}" ]] || return 0
  done
  ANDROID_SERIAL_LIST+=("${serial}")
}

ios_sim_at() {
  local index="$1"
  printf '%s' "${IOS_SIMULATORS}" |
    awk -F'[|,]' -v n="${index}" '{ value = $n; gsub(/^[[:space:]]+|[[:space:]]+$/, "", value); print value }'
}

join_android_avds() {
  local limit="$1"
  local joined=""
  local index=0
  while [[ "${index}" -lt "${limit}" && "${index}" -lt "${#ANDROID_AVD_LIST[@]}" ]]; do
    if [[ -n "${joined}" ]]; then
      joined+=" "
    fi
    joined+="${ANDROID_AVD_LIST[${index}]}"
    index=$((index + 1))
  done
  printf '%s' "${joined}"
}

join_android_serials() {
  local joined=""
  local serial=""
  for serial in ${ANDROID_SERIAL_LIST[@]+"${ANDROID_SERIAL_LIST[@]}"}; do
    if [[ -n "${joined}" ]]; then
      joined+=" "
    fi
    joined+="${serial}"
  done
  printf '%s' "${joined}"
}

max() {
  if [[ "$1" -gt "$2" ]]; then
    printf '%s' "$1"
  else
    printf '%s' "$2"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      LIST_ONLY=1
      shift
      ;;
    --tier)
      append_tiers "$2"
      shift 2
      ;;
    --relay)
      RELAY="$2"
      shift 2
      ;;
    --run-dir)
      RUN_DIR="$2"
      shift 2
      ;;
    --ios-simulators)
      IOS_SIMULATORS="$2"
      shift 2
      ;;
    --android-avds)
      ANDROID_AVDS="$2"
      shift 2
      ;;
    --android-serials)
      ANDROID_SERIALS="$2"
      shift 2
      ;;
    --soak-iterations)
      SOAK_ITERATIONS="$2"
      shift 2
      ;;
    --headless)
      HEADLESS=1
      shift
      ;;
    --wipe-data)
      WIPE_DATA=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --allow-physical)
      ALLOW_PHYSICAL=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
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

if [[ "${LIST_ONLY}" -eq 1 ]]; then
  list_tiers
  exit 0
fi

if [[ "${#TIERS[@]}" -eq 0 ]]; then
  TIERS=(smoke)
fi

case "${RELAY}" in
  local|public) ;;
  *)
    echo "Unknown relay mode: ${RELAY}" >&2
    exit 2
    ;;
esac

if [[ ! "${SOAK_ITERATIONS}" =~ ^[0-9]+$ || "${SOAK_ITERATIONS}" -lt 1 ]]; then
  echo "--soak-iterations must be a positive integer." >&2
  exit 2
fi

NEEDED_AVDS=0
NEEDS_IOS=0
NEEDS_PHYSICAL=0
for tier in "${TIERS[@]}"; do
  case "${tier}" in
    smoke)
      NEEDED_AVDS="$(max "${NEEDED_AVDS}" 2)"
      NEEDS_IOS=1
      ;;
    daily|release)
      NEEDED_AVDS="$(max "${NEEDED_AVDS}" 2)"
      NEEDS_IOS=1
      ;;
    soak)
      NEEDED_AVDS="$(max "${NEEDED_AVDS}" 1)"
      NEEDS_IOS=1
      ;;
    docker)
      ;;
    physical)
      NEEDS_PHYSICAL=1
      NEEDS_IOS=1
      ;;
    *)
      echo "Unknown tier: ${tier}" >&2
      list_tiers >&2
      exit 2
      ;;
  esac
done

while IFS= read -r value; do
  append_unique_avd "${value}"
done < <(split_android_values "${ANDROID_AVDS}")

while IFS= read -r value; do
  append_unique_serial "${value}"
done < <(split_android_values "${ANDROID_SERIALS}")

if [[ "${NEEDS_PHYSICAL}" -eq 1 ]]; then
  if [[ "${ALLOW_PHYSICAL}" != "1" ]]; then
    echo "--tier physical requires --allow-physical because it resets app state on selected phones." >&2
    exit 2
  fi
  if [[ "${#ANDROID_SERIAL_LIST[@]}" -lt 1 ]]; then
    echo "--tier physical requires --android-serials or IRIS_ANDROID_E2E_SERIALS." >&2
    exit 2
  fi
  PHYSICAL_AVDS_NEEDED=$((2 - ${#ANDROID_SERIAL_LIST[@]}))
  if [[ "${PHYSICAL_AVDS_NEEDED}" -gt 0 ]]; then
    NEEDED_AVDS="$(max "${NEEDED_AVDS}" "${PHYSICAL_AVDS_NEEDED}")"
  fi
fi

if [[ "${#ANDROID_AVD_LIST[@]}" -lt "${NEEDED_AVDS}" ]]; then
  if discovered="$("${ROOT_DIR}/scripts/run_android_emulators.sh" --list 2>/dev/null)"; then
    while IFS= read -r avd; do
      append_unique_avd "${avd}"
    done <<<"${discovered}"
  fi
fi

if [[ "${#ANDROID_AVD_LIST[@]}" -lt "${NEEDED_AVDS}" ]]; then
  if [[ "${DRY_RUN}" -eq 1 ]]; then
    while [[ "${#ANDROID_AVD_LIST[@]}" -lt "${NEEDED_AVDS}" ]]; do
      append_unique_avd "Android_AVD_$(( ${#ANDROID_AVD_LIST[@]} + 1 ))"
    done
  else
    echo "Need ${NEEDED_AVDS} Android AVD(s); found ${#ANDROID_AVD_LIST[@]}." >&2
    echo "Create/boot AVDs with scripts/run_android_emulators.sh, or pass --android-avds." >&2
    exit 2
  fi
fi

IOS_SIM_A="$(ios_sim_at 1)"
IOS_SIM_B="$(ios_sim_at 2)"
if [[ "${NEEDS_IOS}" -eq 1 && ( -z "${IOS_SIM_A}" || -z "${IOS_SIM_B}" ) ]]; then
  echo "Need two iOS simulator names for the lab. Pass --ios-simulators 'Name A|Name B'." >&2
  exit 2
fi

REUSE_LOCAL_BUILDS=0
RELIABILITY_RELAY_PORT=""
RELIABILITY_IOS_RELAY_URL=""
RELIABILITY_ANDROID_RELAY_URL=""
if [[ "${RELAY}" == "local" && "${NEEDS_PHYSICAL}" -eq 0 ]]; then
  REUSE_LOCAL_BUILDS=1
  RELIABILITY_RELAY_PORT="${IRIS_E2E_RELIABILITY_RELAY_PORT:-$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)}"
  RELIABILITY_IOS_RELAY_URL="ws://127.0.0.1:${RELIABILITY_RELAY_PORT}"
  RELIABILITY_ANDROID_RELAY_URL="ws://10.0.2.2:${RELIABILITY_RELAY_PORT}"
  export IRIS_E2E_RELAY_SET_ID="${IRIS_E2E_RELAY_SET_ID:-reliability-lab-local}"
fi

mkdir -p "${RUN_DIR}"
LOG_FILE="${RUN_DIR}/reliability-lab.log"
STEPS_FILE="${RUN_DIR}/steps.tsv"
printf 'step\tstatus\tname\tartifact_dir\n' >"${STEPS_FILE}"
iris_e2e_record_repo_trace "${ROOT_DIR}" "${RUN_DIR}" || true
{
  printf 'run_dir=%s\n' "${RUN_DIR}"
  printf 'tiers=%s\n' "${TIERS[*]}"
  printf 'relay=%s\n' "${RELAY}"
  printf 'ios_simulators=%s\n' "${IOS_SIMULATORS}"
  printf 'android_avds=%s\n' "$(join_android_avds "${#ANDROID_AVD_LIST[@]}")"
  printf 'physical_serials=%s\n' "$(join_android_serials)"
  printf 'dry_run=%s\n' "${DRY_RUN}"
  printf 'headless=%s\n' "${HEADLESS}"
  printf 'wipe_data=%s\n' "${WIPE_DATA}"
  printf 'skip_build=%s\n' "${SKIP_BUILD}"
  printf 'reuse_local_builds=%s\n' "${REUSE_LOCAL_BUILDS}"
  printf 'reliability_relay_port=%s\n' "${RELIABILITY_RELAY_PORT}"
} >"${RUN_DIR}/manifest.env"

NO_PHYSICAL_ENV=(env -u IRIS_ANDROID_E2E_SERIALS -u IRIS_ANDROID_E2E_SERIAL -u IRIS_ANDROID_PHONE_SERIAL)
PHYSICAL_ENV=(env IRIS_ANDROID_E2E_SERIALS="$(join_android_serials)")
COMMON_ANDROID_ARGS=()
[[ "${HEADLESS}" -eq 1 ]] && COMMON_ANDROID_ARGS+=(--headless)
[[ "${WIPE_DATA}" -eq 1 ]] && COMMON_ANDROID_ARGS+=(--wipe-data)
COMMON_SKIP_ARGS=()
[[ "${SKIP_BUILD}" -eq 1 ]] && COMMON_SKIP_ARGS+=(--skip-build)
COMMON_IOS_RELAY_ARGS=()
COMMON_ANDROID_RELAY_ARGS=()
COMMON_MIXED_RELAY_ARGS=()
if [[ "${REUSE_LOCAL_BUILDS}" -eq 1 ]]; then
  COMMON_IOS_RELAY_ARGS=(
    --relay-port "${RELIABILITY_RELAY_PORT}"
    --relay-url "${RELIABILITY_IOS_RELAY_URL}"
  )
  COMMON_ANDROID_RELAY_ARGS=(
    --relay-port "${RELIABILITY_RELAY_PORT}"
    --relay-url "${RELIABILITY_ANDROID_RELAY_URL}"
  )
  COMMON_MIXED_RELAY_ARGS=(
    --relay-port "${RELIABILITY_RELAY_PORT}"
    --relay-url "${RELIABILITY_IOS_RELAY_URL}"
    --android-relay-url "${RELIABILITY_ANDROID_RELAY_URL}"
  )
fi

run_step() {
  local name="$1"
  local artifact_dir="$2"
  shift 2
  STEP_COUNT=$((STEP_COUNT + 1))
  mkdir -p "${artifact_dir}"
  echo
  echo "=== ${name} ===" | tee -a "${LOG_FILE}"
  print_command "$@" | tee -a "${LOG_FILE}"
  if [[ "${DRY_RUN}" -eq 1 ]]; then
    printf '%s\t%s\t%s\t%s\n' "${STEP_COUNT}" "dry-run" "${name}" "${artifact_dir}" >>"${STEPS_FILE}"
    return 0
  fi
  if "$@" 2>&1 | tee -a "${LOG_FILE}"; then
    printf '%s\t%s\t%s\t%s\n' "${STEP_COUNT}" "passed" "${name}" "${artifact_dir}" >>"${STEPS_FILE}"
  else
    local status=$?
    printf '%s\t%s\t%s\t%s\n' "${STEP_COUNT}" "failed" "${name}" "${artifact_dir}" >>"${STEPS_FILE}"
    return "${status}"
  fi
}

run_smoke() {
  local avd_two
  avd_two="$(join_android_avds 2)"
  run_step "F15 iOS secret-key restore and message" \
    "${RUN_DIR}/$(printf '%02d-f15-ios-secret-key-restore' "$((STEP_COUNT + 1))")" \
    python3 "${ROOT_DIR}/scripts/ios_restore_existing_profile.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f15-ios-secret-key-restore' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --simulators "${IOS_SIMULATORS}" \
      ${COMMON_IOS_RELAY_ARGS[@]+"${COMMON_IOS_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F06 Android offline restart recovery" \
    "${RUN_DIR}/$(printf '%02d-f06-android-offline-restart' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/android_offline_restart_recovery.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f06-android-offline-restart' "$((STEP_COUNT + 1))")" \
      --avds "${avd_two}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_ANDROID_RELAY_ARGS[@]+"${COMMON_ANDROID_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  # The first two smoke flows populated the shared iOS and Android artifacts.
  # Remaining local emulator flows use the same compiled relay configuration.
  if [[ "${REUSE_LOCAL_BUILDS}" -eq 1 && "${SKIP_BUILD}" -eq 0 ]]; then
    COMMON_SKIP_ARGS=(--skip-build)
  fi

  run_step "F16 mixed cold group invite" \
    "${RUN_DIR}/$(printf '%02d-f16-mixed-cold-group-invite' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_cold_group_invite.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f16-mixed-cold-group-invite' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulator "${IOS_SIM_A}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}
}

run_daily_extra() {
  local avd_two
  avd_two="$(join_android_avds 2)"
  run_step "F15 Android secret-key restore and message" \
    "${RUN_DIR}/$(printf '%02d-f15-android-secret-key-restore' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/android_restore_existing_profile.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f15-android-secret-key-restore' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --avds "${avd_two}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_ANDROID_RELAY_ARGS[@]+"${COMMON_ANDROID_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F06 mixed offline restart recovery" \
    "${RUN_DIR}/$(printf '%02d-f06-mixed-offline-restart' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_offline_restart_recovery.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f06-mixed-offline-restart' "$((STEP_COUNT + 1))")" \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulator "${IOS_SIM_A}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F08 mixed group membership" \
    "${RUN_DIR}/$(printf '%02d-f08-mixed-group-membership' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_group_membership_matrix.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f08-mixed-group-membership' "$((STEP_COUNT + 1))")" \
      --android-avd-only \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulators "${IOS_SIMULATORS}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F12 mixed app parity" \
    "${RUN_DIR}/$(printf '%02d-f12-mixed-app-parity' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_app_parity_flow.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f12-mixed-app-parity' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulator "${IOS_SIM_A}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F10 mixed linked-device revocation" \
    "${RUN_DIR}/$(printf '%02d-f10-mixed-linked-device-revocation' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_linked_device_revocation.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f10-mixed-linked-device-revocation' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --android-avd-only \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulators "${IOS_SIMULATORS}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "F17 mixed multi-device mesh" \
    "${RUN_DIR}/$(printf '%02d-f17-mixed-multi-device-mesh' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f17-mixed-multi-device-mesh' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --android-avd-only \
      --avds "${avd_two}" \
      --simulators "${IOS_SIMULATORS}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}
}

run_soak() {
  run_step "F14 mixed restart soak" \
    "${RUN_DIR}/$(printf '%02d-f14-mixed-restart-soak' "$((STEP_COUNT + 1))")" \
    "${NO_PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_multi_restart_soak.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-f14-mixed-restart-soak' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --android-avd-only \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulators "${IOS_SIMULATORS}" \
      --iterations "${SOAK_ITERATIONS}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_MIXED_RELAY_ARGS[@]+"${COMMON_MIXED_RELAY_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}
}

run_docker() {
  run_step "Docker production CLI relay e2e" \
    "${RUN_DIR}/$(printf '%02d-docker-production-cli-relay-e2e' "$((STEP_COUNT + 1))")" \
    "${ROOT_DIR}/scripts/cli_production_relay_e2e_docker"
}

run_physical() {
  local avd_two
  local serials
  avd_two="$(join_android_avds 2)"
  serials="$(join_android_serials)"
  run_step "physical F06 Android offline restart recovery" \
    "${RUN_DIR}/$(printf '%02d-physical-f06-android-offline-restart' "$((STEP_COUNT + 1))")" \
    "${PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/android_offline_restart_recovery.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-physical-f06-android-offline-restart' "$((STEP_COUNT + 1))")" \
      --serials "${serials}" \
      --avds "${avd_two}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "physical F15 mixed secret-key restore" \
    "${RUN_DIR}/$(printf '%02d-physical-f15-mixed-secret-key-restore' "$((STEP_COUNT + 1))")" \
    "${PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_restore_existing_profile.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-physical-f15-mixed-secret-key-restore' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --serials "${serials}" \
      --avd "${ANDROID_AVD_LIST[0]}" \
      --simulator "${IOS_SIM_A}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}

  run_step "physical F17 mixed multi-device mesh" \
    "${RUN_DIR}/$(printf '%02d-physical-f17-mixed-multi-device-mesh' "$((STEP_COUNT + 1))")" \
    "${PHYSICAL_ENV[@]}" python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py" \
      --artifact-dir "${RUN_DIR}/$(printf '%02d-physical-f17-mixed-multi-device-mesh' "$((STEP_COUNT + 1))")" \
      --relay-mode "${RELAY}" \
      --serials "${serials}" \
      --avds "${avd_two}" \
      --simulators "${IOS_SIMULATORS}" \
      ${COMMON_ANDROID_ARGS[@]+"${COMMON_ANDROID_ARGS[@]}"} \
      ${COMMON_SKIP_ARGS[@]+"${COMMON_SKIP_ARGS[@]}"}
}

echo "iris reliability lab"
echo "run dir: ${RUN_DIR}"
echo "tiers: ${TIERS[*]}"
echo "android avds: $(join_android_avds "${#ANDROID_AVD_LIST[@]}")"
if [[ "${NEEDS_PHYSICAL}" -eq 1 ]]; then
  echo "physical serials: $(join_android_serials)"
else
  echo "physical devices: disabled"
fi

for tier in "${TIERS[@]}"; do
  case "${tier}" in
    smoke)
      run_smoke
      ;;
    daily)
      run_smoke
      run_daily_extra
      ;;
    soak)
      run_soak
      ;;
    docker)
      run_docker
      ;;
    release)
      run_smoke
      run_daily_extra
      run_soak
      run_docker
      ;;
    physical)
      run_physical
      ;;
  esac
done

echo
echo "Reliability lab complete. Artifacts: ${RUN_DIR}"
