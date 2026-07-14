#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_PROPERTIES="${ROOT_DIR}/android/local.properties"
SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
DEFAULT_AVDS=("Medium_Phone_API_36.1" "Pixel_9a" "Pixel_Fold")
DNS_SERVERS="${IRIS_ANDROID_DNS_SERVERS:-8.8.8.8,1.1.1.1}"

if [[ -z "${SDK_DIR}" && -f "${LOCAL_PROPERTIES}" ]]; then
  SDK_DIR="$(sed -n 's/^sdk\.dir=//p' "${LOCAL_PROPERTIES}" | tail -n 1)"
fi

if [[ -z "${SDK_DIR}" ]]; then
  echo "Android SDK path not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or sdk.dir in local.properties." >&2
  exit 1
fi

ADB="${SDK_DIR}/platform-tools/adb"
EMULATOR="${SDK_DIR}/emulator/emulator"

if [[ ! -x "${ADB}" ]]; then
  echo "adb not found at ${ADB}" >&2
  exit 1
fi

if [[ ! -x "${EMULATOR}" ]]; then
  echo "emulator not found at ${EMULATOR}" >&2
  exit 1
fi

HEADLESS=0
WIPE_DATA=0
LIST_ONLY=0
AVDS=()
LAUNCHED_EMULATOR_PID=""
LAUNCHED_EMULATOR_LOG=""
ASSIGNED_SERIALS=()

usage() {
  cat <<'EOF'
Usage: scripts/run_android_emulators.sh [options] [avd...]

Options:
  --headless   Launch emulators without a window
  --wipe-data  Wipe data when launching missing emulators
  --list       Print configured AVD names and exit

Environment:
  IRIS_ANDROID_DNS_SERVERS  Comma-separated DNS servers passed to the emulator.
                           Defaults to 8.8.8.8,1.1.1.1. Set to off to use
                           the emulator's inherited resolver configuration.

Defaults:
  Medium_Phone_API_36.1
  Pixel_9a
  Pixel_Fold
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --headless)
      HEADLESS=1
      shift
      ;;
    --wipe-data)
      WIPE_DATA=1
      shift
      ;;
    --list)
      LIST_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      AVDS+=("$1")
      shift
      ;;
  esac
done

if [[ ${LIST_ONLY} -eq 1 ]]; then
  find "${HOME}/.android/avd" -maxdepth 1 -name '*.ini' -type f -exec basename {} .ini \; | sort
  exit 0
fi

if [[ ${#AVDS[@]} -eq 0 ]]; then
  AVDS=("${DEFAULT_AVDS[@]}")
fi

serial_is_assigned() {
  local candidate="$1"
  local assigned
  for assigned in "${ASSIGNED_SERIALS[@]:-}"; do
    [[ "${assigned}" == "${candidate}" ]] && return 0
  done
  return 1
}

find_serial_for_avd() {
  local avd_name="$1"
  while read -r serial _; do
    [[ -z "${serial}" || "${serial}" == "List" ]] && continue
    local running_name
    running_name="$("${ADB}" -s "${serial}" emu avd name 2>/dev/null | tr -d '\r' | head -n 1 || true)"
    if [[ "${running_name}" == "${avd_name}" ]] && ! serial_is_assigned "${serial}"; then
      echo "${serial}"
      return 0
    fi
  done < <("${ADB}" devices | awk 'NR>1 { print $1, $2 }')
  return 1
}

avd_is_running() {
  local avd_name="$1"
  while read -r serial _; do
    [[ -z "${serial}" || "${serial}" == "List" ]] && continue
    local running_name
    running_name="$("${ADB}" -s "${serial}" emu avd name 2>/dev/null | tr -d '\r' | head -n 1 || true)"
    [[ "${running_name}" == "${avd_name}" ]] && return 0
  done < <("${ADB}" devices | awk 'NR>1 { print $1, $2 }')
  return 1
}

avd_requires_read_only() {
  local avd_name="$1"
  local requested count=0
  for requested in "${AVDS[@]}"; do
    [[ "${requested}" == "${avd_name}" ]] && count=$((count + 1))
  done
  [[ "${count}" -gt 1 ]]
}

stop_running_avd_instances() {
  local avd_name="$1"
  local serial running_name
  while read -r serial _; do
    [[ -z "${serial}" || "${serial}" == "List" ]] && continue
    running_name="$("${ADB}" -s "${serial}" emu avd name 2>/dev/null | tr -d '\r' | head -n 1 || true)"
    if [[ "${running_name}" == "${avd_name}" ]]; then
      "${ADB}" -s "${serial}" emu kill >/dev/null 2>&1 || true
    fi
  done < <("${ADB}" devices | awk 'NR>1 { print $1, $2 }')

  for _ in {1..30}; do
    avd_is_running "${avd_name}" || return 0
    sleep 1
  done
  echo "Timed out stopping existing ${avd_name} instances for read-only launch." >&2
  return 1
}

avd_exists() {
  local avd_name="$1"
  local available
  available="$("${EMULATOR}" -list-avds 2>/dev/null || true)"
  while IFS= read -r name; do
    if [[ "${name}" == "${avd_name}" ]]; then
      return 0
    fi
  done <<<"${available}"
  return 1
}

launch_visible_avd() {
  local avd_name="$1"
  local read_only="$2"
  local cmd="\"${EMULATOR}\" -avd \"${avd_name}\" -gpu swiftshader_indirect"
  if [[ "${read_only}" -eq 1 ]]; then
    cmd="${cmd} -read-only -no-snapshot"
  fi
  if [[ -n "${DNS_SERVERS}" && "${DNS_SERVERS}" != "off" ]]; then
    cmd="${cmd} -dns-server \"${DNS_SERVERS}\""
  fi
  if [[ ${WIPE_DATA} -eq 1 ]]; then
    cmd="${cmd} -wipe-data"
  fi
  local escaped="${cmd//\\/\\\\}"
  escaped="${escaped//\"/\\\"}"
  osascript -e "tell application \"Terminal\" to activate" \
    -e "tell application \"Terminal\" to do script \"${escaped}\"" >/dev/null
}

launch_headless_avd() {
  local avd_name="$1"
  local read_only="$2"
  local log_file="/tmp/${avd_name//[^A-Za-z0-9_.-]/_}.log"
  local args=("${EMULATOR}" -avd "${avd_name}" -no-window -no-audio -gpu swiftshader_indirect)
  if [[ "${read_only}" -eq 1 ]]; then
    args+=(-read-only -no-snapshot)
  fi
  if [[ -n "${DNS_SERVERS}" && "${DNS_SERVERS}" != "off" ]]; then
    args+=(-dns-server "${DNS_SERVERS}")
  fi
  if [[ ${WIPE_DATA} -eq 1 ]]; then
    args+=(-wipe-data)
  fi
  : >"${log_file}"
  nohup "${args[@]}" >"${log_file}" 2>&1 &
  LAUNCHED_EMULATOR_PID="$!"
  LAUNCHED_EMULATOR_LOG="${log_file}"
}

ensure_avd_running() {
  local avd_name="$1"
  if ! avd_exists "${avd_name}"; then
    echo "Android AVD ${avd_name} is not installed." >&2
    echo "Installed AVDs:" >&2
    "${EMULATOR}" -list-avds >&2 || true
    return 1
  fi

  local serial
  serial="$(find_serial_for_avd "${avd_name}" || true)"
  if [[ -z "${serial}" ]]; then
    local read_only=0
    if avd_requires_read_only "${avd_name}" || avd_is_running "${avd_name}"; then
      read_only=1
    fi
    LAUNCHED_EMULATOR_PID=""
    LAUNCHED_EMULATOR_LOG=""
    if [[ ${HEADLESS} -eq 1 ]]; then
      launch_headless_avd "${avd_name}" "${read_only}"
    else
      launch_visible_avd "${avd_name}" "${read_only}"
    fi
  fi

  for _ in {1..180}; do
    serial="$(find_serial_for_avd "${avd_name}" || true)"
    if [[ -n "${serial}" ]]; then
      local booted
      booted="$("${ADB}" -s "${serial}" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' || true)"
      if [[ "${booted}" == "1" ]]; then
        echo "${serial}"
        return 0
      fi
    elif [[ -n "${LAUNCHED_EMULATOR_PID}" ]] && ! kill -0 "${LAUNCHED_EMULATOR_PID}" 2>/dev/null; then
      echo "Emulator ${avd_name} exited during startup." >&2
      if [[ -n "${LAUNCHED_EMULATOR_LOG}" && -f "${LAUNCHED_EMULATOR_LOG}" ]]; then
        tail -80 "${LAUNCHED_EMULATOR_LOG}" >&2 || true
      fi
      return 1
    elif [[ -n "${LAUNCHED_EMULATOR_LOG}" && -f "${LAUNCHED_EMULATOR_LOG}" ]] && grep -Eq 'FATAL|not enough disk space' "${LAUNCHED_EMULATOR_LOG}"; then
      echo "Emulator ${avd_name} reported a startup failure." >&2
      tail -80 "${LAUNCHED_EMULATOR_LOG}" >&2 || true
      if [[ -n "${LAUNCHED_EMULATOR_PID}" ]]; then
        kill "${LAUNCHED_EMULATOR_PID}" 2>/dev/null || true
      fi
      return 1
    fi
    sleep 2
  done

  echo "Timed out waiting for ${avd_name} to boot." >&2
  return 1
}

prepared_read_only_avds=()
for avd_name in "${AVDS[@]}"; do
  if avd_requires_read_only "${avd_name}"; then
    already_prepared=0
    for prepared in "${prepared_read_only_avds[@]:-}"; do
      [[ "${prepared}" == "${avd_name}" ]] && already_prepared=1
    done
    if [[ "${already_prepared}" -eq 0 ]]; then
      stop_running_avd_instances "${avd_name}"
      prepared_read_only_avds+=("${avd_name}")
    fi
  fi
done

for avd_name in "${AVDS[@]}"; do
  serial="$(ensure_avd_running "${avd_name}")"
  ASSIGNED_SERIALS+=("${serial}")
  echo "${avd_name} ${serial}"
done
