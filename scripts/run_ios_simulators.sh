#!/usr/bin/env bash

set -Eeuo pipefail

DEFAULT_SIMULATORS=("Iris Chat iPhone" "Iris Chat iPhone 2")
LIST_ONLY=0
NO_OPEN=0
UDIDS_ONLY=0
SIMULATORS=()

usage() {
  cat <<'EOF'
Usage: scripts/run_ios_simulators.sh [options] [simulator-name...]

Options:
  --list     Print available simulators and runtimes, then exit
  --no-open  Do not open the Simulator app after booting
  --udids-only  Print one booted simulator UDID per line

Defaults:
  Iris Chat iPhone
  Iris Chat iPhone 2
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      LIST_ONLY=1
      shift
      ;;
    --no-open)
      NO_OPEN=1
      shift
      ;;
    --udids-only)
      UDIDS_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      SIMULATORS+=("$1")
      shift
      ;;
  esac
done

if ! command -v xcrun >/dev/null 2>&1; then
  echo "xcrun not found. Install Xcode command line tools." >&2
  exit 1
fi

if [[ ${LIST_ONLY} -eq 1 ]]; then
  xcrun simctl list runtimes available
  echo "---"
  xcrun simctl list devices available
  exit 0
fi

if [[ ${#SIMULATORS[@]} -eq 0 ]]; then
  SIMULATORS=("${DEFAULT_SIMULATORS[@]}")
fi

SETUP_RAW="$(
  xcrun simctl list -j devicetypes runtimes devices | python3 -c '
import json
import re
import sys

data = json.load(sys.stdin)
runtimes = [
    runtime for runtime in data.get("runtimes", [])
    if runtime.get("isAvailable") and runtime.get("identifier", "").startswith("com.apple.CoreSimulator.SimRuntime.iOS")
]
if not runtimes:
    raise SystemExit("NO_IOS_RUNTIME")

def version_key(runtime):
    identifier = runtime.get("identifier", "")
    match = re.search(r"iOS[- ](.+)$", runtime.get("name", "")) or re.search(r"iOS-(.+)$", identifier)
    parts = re.findall(r"\d+", match.group(1) if match else "")
    return tuple(int(part) for part in parts) if parts else (0,)

runtime = max(runtimes, key=version_key)
preferred = ["iPhone 16", "iPhone 16 Pro", "iPhone 15", "iPhone 14"]
device_types = data.get("devicetypes", [])
device_type = None
for name in preferred:
    device_type = next((item for item in device_types if item.get("name") == name), None)
    if device_type is not None:
        break
if device_type is None:
    device_type = next((item for item in device_types if "iPhone" in item.get("name", "")), None)
if device_type is None:
    raise SystemExit("NO_IPHONE_DEVICE_TYPE")

print(runtime["identifier"])
print(device_type["identifier"])
print(device_type["name"])
'
)"

SETUP=()
while IFS= read -r line; do
  SETUP+=("${line}")
done <<< "${SETUP_RAW}"

if [[ ${#SETUP[@]} -lt 3 ]]; then
  echo "No available iOS simulator runtime found. Install an iOS runtime in Xcode Settings > Components and try again." >&2
  exit 1
fi

RUNTIME_ID="${SETUP[0]}"
DEVICE_TYPE_ID="${SETUP[1]}"
DEVICE_TYPE_NAME="${SETUP[2]}"

find_device_udids() {
  local simulator_name="$1"
  xcrun simctl list -j devices | python3 -c '
import json
import sys

runtime_id = sys.argv[1]
name = sys.argv[2]
data = json.load(sys.stdin)
for device in data.get("devices", {}).get(runtime_id, []):
    if device.get("name") == name and device.get("isAvailable", True):
        print(device.get("udid", ""))
' "${RUNTIME_ID}" "${simulator_name}"
}

shutdown_stale_ios_simulators() {
  if [[ "${IRIS_E2E_CLOSE_STALE_IOS_SIMS:-1}" == "0" ]]; then
    return 0
  fi

  local -a keep=("$@")
  local udid=""
  while IFS= read -r udid; do
    local keep_udid=""
    local should_keep=0
    for keep_udid in "${keep[@]}"; do
      if [[ "${udid}" == "${keep_udid}" ]]; then
        should_keep=1
        break
      fi
    done
    if [[ "${should_keep}" -eq 0 ]]; then
      if pgrep -fl xcodebuild 2>/dev/null | grep -F "id=${udid}" >/dev/null 2>&1; then
        echo "Keeping active iOS simulator ${udid}" >&2
        continue
      fi
      echo "Shutting down stale iOS simulator ${udid}" >&2
      xcrun simctl shutdown "${udid}" >/dev/null 2>&1 || true
    fi
  done < <(xcrun simctl list devices booted | sed -n 's/.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p')
  quit_idle_ios_simulator_app
}

quit_idle_ios_simulator_app() {
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
    return 0
  fi
  if xcrun simctl list devices booted | grep -q "(Booted)"; then
    return 0
  fi
  if pgrep -fl xcodebuild 2>/dev/null | grep -E "id=[0-9A-F-]{36}|platform=iOS Simulator|iphonesimulator" >/dev/null 2>&1; then
    return 0
  fi
  if ! pgrep -x Simulator >/dev/null 2>&1; then
    return 0
  fi
  echo "Quitting idle iOS Simulator app" >&2
  osascript -e 'tell application "Simulator" to quit' >/dev/null 2>&1 ||
    pkill -x Simulator >/dev/null 2>&1 ||
    true
}

wait_for_bootstatus() {
  local udid="$1"
  local timeout_secs="${IRIS_IOS_BOOTSTATUS_TIMEOUT_SECS:-120}"
  local fallback_sleep="${IRIS_IOS_BOOTSTATUS_FALLBACK_SLEEP_SECS:-20}"
  local deadline=$((SECONDS + timeout_secs))
  local pid=""

  xcrun simctl bootstatus "${udid}" -b >/dev/null 2>&1 &
  pid=$!
  while kill -0 "${pid}" >/dev/null 2>&1; do
    if (( SECONDS >= deadline )); then
      kill "${pid}" >/dev/null 2>&1 || true
      for _ in 1 2 3 4 5; do
        if ! kill -0 "${pid}" >/dev/null 2>&1; then
          break
        fi
        sleep 1
      done
      if kill -0 "${pid}" >/dev/null 2>&1; then
        kill -9 "${pid}" >/dev/null 2>&1 || true
      fi
      wait "${pid}" >/dev/null 2>&1 || true
      if xcrun simctl list devices booted | grep -q "(${udid}) (Booted)"; then
        echo "Timed out waiting for iOS simulator ${udid} bootstatus; continuing because simctl reports Booted." >&2
        sleep "${fallback_sleep}"
        return 0
      fi
      echo "Timed out waiting for iOS simulator ${udid} to boot." >&2
      return 1
    fi
    sleep 1
  done

  if wait "${pid}"; then
    return 0
  fi
  if xcrun simctl list devices booted | grep -q "(${udid}) (Booted)"; then
    echo "iOS simulator ${udid} bootstatus failed; continuing because simctl reports Booted." >&2
    sleep "${fallback_sleep}"
    return 0
  fi
  return 1
}

boot_device() {
  local udid="$1"
  local boot_output=""
  local boot_status=0

  if boot_output="$(xcrun simctl boot "${udid}" 2>&1)"; then
    boot_status=0
  else
    boot_status=$?
  fi
  if [[ "${boot_status}" -ne 0 ]]; then
    if xcrun simctl list devices booted | grep -q "(${udid}) (Booted)"; then
      return 0
    fi
    printf '%s\n' "${boot_output}" >&2
    if grep -qi "deleted device" <<< "${boot_output}"; then
      return 44
    fi
    return "${boot_status}"
  fi
  wait_for_bootstatus "${udid}"
}

TARGET_UDIDS=()
for simulator_name in "${SIMULATORS[@]}"; do
  udid=""
  while IFS= read -r candidate_udid; do
    [[ -n "${candidate_udid}" ]] || continue
    if boot_device "${candidate_udid}"; then
      udid="${candidate_udid}"
      break
    else
      boot_status=$?
    fi
    if [[ "${boot_status}" -eq 44 ]]; then
      echo "Ignoring deleted iOS simulator record ${candidate_udid}" >&2
    else
      echo "Skipping unbootable iOS simulator ${candidate_udid}" >&2
    fi
  done < <(find_device_udids "${simulator_name}")

  if [[ -z "${udid}" ]]; then
    udid="$(xcrun simctl create "${simulator_name}" "${DEVICE_TYPE_ID}" "${RUNTIME_ID}")"
    echo "Created iOS simulator ${simulator_name} (${udid})" >&2
    if ! boot_device "${udid}"; then
      echo "Retrying first boot for new iOS simulator ${udid}" >&2
      sleep 1
      boot_device "${udid}"
    fi
  fi
  TARGET_UDIDS+=("${udid}")
done

shutdown_stale_ios_simulators "${TARGET_UDIDS[@]}"

for index in "${!SIMULATORS[@]}"; do
  simulator_name="${SIMULATORS[$index]}"
  udid="${TARGET_UDIDS[$index]}"

  if [[ ${UDIDS_ONLY} -eq 1 ]]; then
    echo "${udid}"
  else
    echo "${simulator_name} ${udid} ${DEVICE_TYPE_NAME} ${RUNTIME_ID}"
  fi
done

if [[ ${NO_OPEN} -eq 0 ]]; then
  open -a Simulator >/dev/null 2>&1 || true
fi
