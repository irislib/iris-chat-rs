#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/e2e_prerelease_common.sh"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

IOS_HARNESS="${ROOT_DIR}/scripts/run_ios_harness.py"
ONLY_TEST="IrisChatUITests/AppStoreReviewUITests/testIncomingMessageRequestCanBeBlockedAndReported"
SIMULATOR_NAME="${IRIS_APP_STORE_REVIEW_SIMULATOR:-Iris Chat iPhone}"
UDID="${IRIS_APP_STORE_REVIEW_UDID:-}"
RUN_ID="${IRIS_APP_STORE_REVIEW_RUN_ID:-app-store-review-$(iris_e2e_stamp)}"
UI_RUN_ID="harness-${RUN_ID}"
MESSAGE="${IRIS_APP_STORE_REVIEW_MESSAGE:-app-store-review-${RUN_ID}}"
RELAY_SET_ID="${IRIS_APP_STORE_REVIEW_RELAY_SET_ID:-$(local_relay_set_id)}"
RELAYS="${IRIS_APP_STORE_REVIEW_RELAYS:-$(local_ios_relay_url)}"
RUN_DIR="${IRIS_APP_STORE_REVIEW_RUN_DIR:-/tmp/iris-chat-app-store-review-${RUN_ID}}"
DERIVED_DATA="${RUN_DIR}/ui-derived-data"
SENDER_DIR="${RUN_DIR}/rust-sender"
LOG_FILE="${RUN_DIR}/app-store-review.log"

usage() {
  cat <<'EOF'
usage: scripts/e2e_ios_app_store_review_block_report.sh [options]

Runs the iOS App Store UGC review flow on a simulator:
  1. A fresh iOS user creates a profile.
  2. A separate Rust CLI user sends them a direct message through a local message server.
  3. The iOS UI opens the message request and verifies block + report controls.

Options:
  --simulator NAME   Simulator name. Default: Iris Chat iPhone.
  --udid UDID        Simulator UDID. Overrides --simulator.
  --run-id ID        Stable run id for app/harness storage.
  --message TEXT     Incoming message body.
  --run-dir PATH     Output directory.
  -h, --help         Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --simulator)
      SIMULATOR_NAME="$2"
      shift 2
      ;;
    --udid)
      UDID="$2"
      shift 2
      ;;
    --run-id)
      RUN_ID="$2"
      UI_RUN_ID="harness-${RUN_ID}"
      shift 2
      ;;
    --message)
      MESSAGE="$2"
      shift 2
      ;;
    --run-dir)
      RUN_DIR="$2"
      DERIVED_DATA="${RUN_DIR}/ui-derived-data"
      SENDER_DIR="${RUN_DIR}/rust-sender"
      LOG_FILE="${RUN_DIR}/app-store-review.log"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

mkdir -p "${RUN_DIR}"
: >"${LOG_FILE}"

log() {
  printf '%s\n' "$*" | tee -a "${LOG_FILE}" >&2
}

resolve_udid() {
  if [[ -n "${UDID}" ]]; then
    printf '%s\n' "${UDID}"
    return
  fi
  "${ROOT_DIR}/scripts/run_ios_simulators.sh" --no-open "${SIMULATOR_NAME}" >/dev/null
  xcrun simctl list -j devices available | python3 -c '
import json
import sys

name = sys.argv[1]
data = json.load(sys.stdin)
for devices in data.get("devices", {}).values():
    for device in devices:
        if device.get("name") == name:
            print(device.get("udid", ""))
            raise SystemExit(0)
raise SystemExit(f"Simulator `{name}` was not found.")
' "${SIMULATOR_NAME}"
}

find_xctestrun() {
  find "${DERIVED_DATA}/Build/Products" -name '*.xctestrun' -print |
    grep iphonesimulator |
    sort |
    head -n 1
}

prepare_ui_xctestrun() {
  local phase="$1"
  local target="${XCTESTRUN%.xctestrun}.${phase}.xctestrun"
  python3 - "${XCTESTRUN}" "${target}" "${phase}" "${UI_RUN_ID}" "${MESSAGE}" <<'PY'
import plistlib
import shutil
import sys

source, target, phase, run_id, message = sys.argv[1:]
shutil.copy2(source, target)

with open(target, "rb") as handle:
    data = plistlib.load(handle)

def iter_targets(root):
    if "TestConfigurations" in root:
        for test_configuration in root.get("TestConfigurations", []):
            yield from test_configuration.get("TestTargets", [])
        return

    for key, value in root.items():
        if key == "__xctestrun_metadata__":
            continue
        if isinstance(value, dict):
            yield value

test_env = {
    "IRIS_UI_TEST_APP_STORE_REVIEW_PHASE": phase,
    "IRIS_UI_TEST_APP_STORE_REVIEW_RUN_ID": run_id,
    "IRIS_UI_TEST_APP_STORE_REVIEW_MESSAGE": message,
}
app_env = {
    "IRIS_UI_TEST_RUN_ID": run_id,
    "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1",
    "IRIS_DISABLE_NOTIFICATIONS": "1",
}

found = False
for target_config in iter_targets(data):
    if target_config.get("BlueprintName") != "IrisChatUITests":
        continue
    found = True

    environment = dict(target_config.get("EnvironmentVariables", {}))
    environment.update(test_env)
    target_config["EnvironmentVariables"] = environment

    testing_environment = dict(target_config.get("TestingEnvironmentVariables", {}))
    testing_environment.update(test_env)
    target_config["TestingEnvironmentVariables"] = testing_environment

    ui_environment = dict(target_config.get("UITargetAppEnvironmentVariables", {}))
    ui_environment.update(app_env)
    target_config["UITargetAppEnvironmentVariables"] = ui_environment

if not found:
    raise SystemExit("Unable to find IrisChatUITests target in xctestrun file.")

with open(target, "wb") as handle:
    plistlib.dump(data, handle)

print(target)
PY
}

run_ui_phase() {
  local phase="$1"
  local phase_xctestrun
  phase_xctestrun="$(prepare_ui_xctestrun "${phase}")"
  log "Running UI phase: ${phase}"
  xcodebuild test-without-building \
    -xctestrun "${phase_xctestrun}" \
    -destination "id=${UDID}" \
    -only-testing:"${ONLY_TEST}" 2>&1 | tee -a "${LOG_FILE}"
}

run_harness() {
  local action="$1"
  local rebuild="$2"
  shift 2
  local cmd=(
    python3
    "${IOS_HARNESS}"
    --udid "${UDID}"
    --run-id "${RUN_ID}"
    --action "${action}"
    --data-root "${RUN_DIR}/harness-data"
  )
  [[ "${rebuild}" == "1" ]] && cmd+=(--rebuild)
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  "${cmd[@]}" 2>&1 | tee -a "${LOG_FILE}"
}

extract_json_field() {
  local field="$1"
  python3 -c '
import json
import sys

field = sys.argv[1]
data = json.load(sys.stdin)
value = data.get(field, "")
if not value and isinstance(data.get("data"), dict):
    value = data["data"].get(field, "")
print(value)
' "${field}"
}

build_iris_cli() {
  log "Building Rust CLI sender"
  env \
    IRIS_DEFAULT_RELAYS="${RELAYS}" \
    IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
    cargo build --manifest-path "${ROOT_DIR}/core/Cargo.toml" --bin iris 2>&1 | tee -a "${LOG_FILE}" >&2
  cargo metadata \
    --manifest-path "${ROOT_DIR}/core/Cargo.toml" \
    --format-version 1 \
    --no-deps |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug/iris")'
}

run_iris() {
  env \
    IRIS_DEFAULT_RELAYS="${RELAYS}" \
    IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
    "${IRIS_BIN}" --json --data-dir "${SENDER_DIR}" "$@"
}

send_rust_dm() {
  local recipient_npub="$1"
  mkdir -p "${SENDER_DIR}"
  local sender_identity
  sender_identity="$(run_iris account create --name "Rust Sender")"
  SENDER_NPUB="$(printf '%s\n' "${sender_identity}" | extract_json_field npub)"
  iris_e2e_require_value "sender_npub" "${SENDER_NPUB}"

  log "Rust sender: ${SENDER_NPUB}"
  run_iris sync --wait-ms 8000 >/dev/null

  local attempt
  for attempt in 1 2 3 4 5; do
    if run_iris chat create "${recipient_npub}" >/dev/null &&
       run_iris send "${recipient_npub}" "${MESSAGE}" >/dev/null; then
      run_iris sync --wait-ms 12000 >/dev/null || true
      return 0
    fi
    log "Rust DM send attempt ${attempt} failed; retrying after sync"
    run_iris sync --wait-ms 8000 >/dev/null || true
    sleep 2
  done
  echo "Rust sender could not send the DM" >&2
  return 1
}

RELAY_PID=""
cleanup() {
  if [[ -n "${RELAY_PID}" ]]; then
    stop_local_rust_relay "${RELAY_PID}"
  fi
}
trap cleanup EXIT

UDID="$(resolve_udid)"
log "Simulator UDID: ${UDID}"
log "Run dir: ${RUN_DIR}"
log "Relay: ${RELAYS}"

RELAY_LOG="${RUN_DIR}/local-relay.log"
RELAY_PID="$(start_local_rust_relay "${RELAY_LOG}")"
log "Local relay PID: ${RELAY_PID}"

log "Building iOS app against local message server"
(
  cd "${ROOT_DIR}"
  IRIS_DEFAULT_RELAYS="${RELAYS}" \
  IRIS_RELAY_SET_ID="${RELAY_SET_ID}" \
    ./scripts/ios-build ios-xcframework
  ./scripts/ios-build ios-xcodeproj
)

log "Building UI test bundle"
xcodebuild \
  -project "${ROOT_DIR}/ios/IrisChat.xcodeproj" \
  -scheme IrisChat \
  -destination "id=${UDID}" \
  -derivedDataPath "${DERIVED_DATA}" \
  build-for-testing 2>&1 | tee -a "${LOG_FILE}"

XCTESTRUN="$(find_xctestrun)"
iris_e2e_require_value "xctestrun" "${XCTESTRUN}"

run_ui_phase create_profile

IDENTITY_OUTPUT="$(run_harness report_logged_in_identity 1 \
  wait_for_relay_drain true \
  relay_drain_timeout_secs 180)"
RECIPIENT_NPUB="$(printf '%s\n' "${IDENTITY_OUTPUT}" | iris_e2e_extract_status npub)"
iris_e2e_require_value "recipient_npub" "${RECIPIENT_NPUB}"
log "iOS recipient: ${RECIPIENT_NPUB}"

IRIS_BIN="$(build_iris_cli)"
send_rust_dm "${RECIPIENT_NPUB}"

run_harness wait_for_message_from_args 0 \
  peer_input "${SENDER_NPUB}" \
  message "${MESSAGE}" \
  direction incoming >/dev/null

run_ui_phase block_report

log "App Store review block/report e2e passed"
