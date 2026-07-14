#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORM=auto
SKIP_BUILD=0
ARTIFACT_ROOT="${IRIS_CHAT_IDLE_CPU_ARTIFACT_ROOT:-$ROOT/artifacts/idle-cpu}"
RUN_ID="idle-cpu-$(date -u +%Y%m%dT%H%M%SZ)-$$"
PEER_HEX="1111111111111111111111111111111111111111111111111111111111111111"
APP_PID=""
DISPLAY_PID=""

usage() {
  cat <<'EOF'
usage: scripts/idle-cpu-platform-gate.sh --platform macos|linux|ios|android|windows [--skip-build]

Launches the real app shell with an isolated logged-in fixture containing a
direct chat and a group chat, then blocks when average idle CPU exceeds the
platform limit. Defaults: 30s settle, 60s sample, 5% of one core.

Environment:
  IRIS_CHAT_IDLE_CPU_MAX_PERCENT
  IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS
  IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS
  IRIS_CHAT_IDLE_CPU_ARTIFACT_ROOT
  IRIS_ANDROID_SERIAL
  IRIS_IOS_SIMULATOR / IRIS_IOS_UDID
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform) PLATFORM="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$PLATFORM" == auto ]]; then
  case "$(uname -s)" in
    Darwin) PLATFORM=macos ;;
    Linux) PLATFORM=linux ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORM=windows ;;
    *) echo "Cannot infer platform from $(uname -s)" >&2; exit 2 ;;
  esac
fi

RUN_DIR="$ARTIFACT_ROOT/$PLATFORM-$RUN_ID"
DATA_DIR="$RUN_DIR/data"
FIXTURE="$RUN_DIR/fixture.json"
RESULT="$RUN_DIR/result.json"
mkdir -p "$RUN_DIR"

cleanup() {
  if [[ -n "$APP_PID" ]]; then
    kill "$APP_PID" >/dev/null 2>&1 || true
    wait "$APP_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$DISPLAY_PID" ]]; then
    kill "$DISPLAY_PID" >/dev/null 2>&1 || true
    wait "$DISPLAY_PID" >/dev/null 2>&1 || true
  fi
  if [[ -f "$DATA_DIR/relay.log" ]]; then
    cp "$DATA_DIR/relay.log" "$RUN_DIR/fixture-relay.log"
  fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

write_mobile_fixture() {
  python3 - "$FIXTURE" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({
    "loggedIn": True,
    "directChatCount": 1,
    "groupChatCount": 1,
}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

run_sampler() {
  local mode="$1"
  shift
  "$ROOT/scripts/idle-cpu-gate.py" "$mode" "$@" \
    --fixture "$FIXTURE" \
    --artifact "$RESULT" \
    --max-percent "${IRIS_CHAT_IDLE_CPU_MAX_PERCENT:-5}" \
    --settle-seconds "${IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS:-30}" \
    --sample-seconds "${IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS:-60}"
}

run_macos() {
  [[ "$(uname -s)" == Darwin ]] || { echo "macOS idle CPU gate requires macOS" >&2; exit 1; }
  [[ "$SKIP_BUILD" -eq 1 ]] || "$ROOT/scripts/macos-build" macos-build
  "$ROOT/scripts/seed-idle-cpu-fixture.sh" --data-dir "$DATA_DIR" --secret-format apple
  cp "$DATA_DIR/fixture.json" "$FIXTURE"
  local app="${IRIS_MACOS_IDLE_CPU_APP:-$ROOT/macos/.build/DerivedData/Build/Products/Debug/Iris Chat.app}"
  local executable="$app/Contents/MacOS/Iris Chat"
  [[ -x "$executable" ]] || { echo "Missing macOS app executable: $executable" >&2; exit 1; }
  env \
    IRIS_UI_TEST_RUN_ID="$RUN_ID" \
    IRIS_UI_TEST_DATA_DIR="$DATA_DIR" \
    IRIS_UI_TEST_BYPASS_KEYCHAIN=1 \
    IRIS_DISABLE_NOTIFICATIONS=1 \
    "$executable" >"$RUN_DIR/app.log" 2>&1 &
  APP_PID=$!
  run_sampler host-pid --pid "$APP_PID" --label "macOS Iris Chat"
}

run_linux() {
  [[ "$(uname -s)" == Linux ]] || { echo "Linux idle CPU gate requires Linux" >&2; exit 1; }
  [[ "$SKIP_BUILD" -eq 1 ]] || cargo build --locked --manifest-path "$ROOT/linux/Cargo.toml"
  "$ROOT/scripts/seed-idle-cpu-fixture.sh" --data-dir "$DATA_DIR" --secret-format linux
  cp "$DATA_DIR/fixture.json" "$FIXTURE"
  local app="${IRIS_LINUX_IDLE_CPU_APP:-$ROOT/linux/target/debug/iris-chat}"
  [[ -x "$app" ]] || { echo "Missing Linux app executable: $app" >&2; exit 1; }
  if [[ -z "${DISPLAY:-}" ]]; then
    command -v Xvfb >/dev/null 2>&1 || { echo "Linux idle CPU gate needs DISPLAY or Xvfb" >&2; exit 1; }
    local display_number="${IRIS_CHAT_IDLE_CPU_XVFB_DISPLAY:-:97}"
    Xvfb "$display_number" -screen 0 1280x800x24 >"$RUN_DIR/xvfb.log" 2>&1 &
    DISPLAY_PID=$!
    export DISPLAY="$display_number"
    sleep 1
  fi
  env \
    IRIS_UI_TEST_RUN_ID="$RUN_ID" \
    IRIS_UI_TEST_DATA_DIR="$DATA_DIR" \
    "$app" >"$RUN_DIR/app.log" 2>&1 &
  APP_PID=$!
  run_sampler host-pid --pid "$APP_PID" --label "Linux Iris Chat"
}

android_adb() {
  if [[ -n "${ADB:-}" ]]; then printf '%s\n' "$ADB"; return; fi
  if command -v adb >/dev/null 2>&1; then command -v adb; return; fi
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  [[ -z "$sdk" ]] || printf '%s\n' "$sdk/platform-tools/adb"
}

run_android_harness() {
  local adb="$1" serial="$2" test_name="$3"
  shift 3
  local cmd=(python3 "$ROOT/scripts/run_harness.py" \
    --adb "$adb" --serial "$serial" \
    --runner to.iris.chat.test/androidx.test.runner.AndroidJUnitRunner \
    --class-name to.iris.chat.RealRelayHarnessTest \
    --test-name "$test_name")
  while [[ $# -gt 0 ]]; do cmd+=(--arg "$1=$2"); shift 2; done
  "${cmd[@]}"
}

run_android() {
  local adb serial package="${IRIS_CHAT_ANDROID_PACKAGE:-to.iris.chat.debug}"
  adb="$(android_adb)"
  [[ -x "$adb" ]] || { echo "adb not found" >&2; exit 1; }
  serial="${IRIS_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"
  if [[ -z "$serial" ]]; then
    serial="$($adb devices | awk 'NR > 1 && $2 == "device" && $1 ~ /^emulator-/ { print $1; exit }')"
  fi
  [[ -n "$serial" ]] || {
    echo "No Android emulator is online; set IRIS_ANDROID_SERIAL explicitly to authorize a physical device" >&2
    exit 1
  }
  if [[ "$SKIP_BUILD" -eq 0 ]]; then
    (cd "$ROOT/android" && ANDROID_SERIAL="$serial" ./gradlew :app:installDebug :app:installDebugAndroidTest)
  fi
  "$adb" -s "$serial" shell pm clear "$package" >/dev/null
  "$adb" -s "$serial" shell pm clear to.iris.chat.test >/dev/null 2>&1 || true
  run_android_harness "$adb" "$serial" create_account_and_report_identity
  run_android_harness "$adb" "$serial" create_chat_from_args peer_input "$PEER_HEX"
  run_android_harness "$adb" "$serial" create_group_from_args group_name "Idle CPU group"
  write_mobile_fixture
  "$adb" -s "$serial" shell am force-stop "$package"
  "$adb" -s "$serial" shell am start -W -n "$package/to.iris.chat.MainActivity" >/dev/null
  run_sampler android-package --adb "$adb" --serial "$serial" --package "$package" --label "Android Iris Chat"
}

ios_udid() {
  if [[ -n "${IRIS_IOS_UDID:-}" ]]; then printf '%s\n' "$IRIS_IOS_UDID"; return; fi
  local simulator="${IRIS_IOS_SIMULATOR:-Iris Chat iPhone}"
  xcrun simctl list devices available | sed -n "s/.*${simulator} (\([0-9A-F-]\{36\}\)).*/\1/p" | head -n 1
}

ios_simulator_is_booted() {
  local udid="$1"
  xcrun simctl list devices booted | grep -Fq "($udid) (Booted)"
}

run_ios_harness() {
  local udid="$1" action="$2"
  shift 2
  local cmd=(python3 "$ROOT/scripts/run_ios_harness.py" --udid "$udid" --run-id "$RUN_ID" \
    --action "$action")
  while [[ $# -gt 0 ]]; do cmd+=(--arg "$1=$2"); shift 2; done
  "${cmd[@]}"
}

run_ios() {
  [[ "$(uname -s)" == Darwin ]] || { echo "iOS idle CPU gate requires macOS" >&2; exit 1; }
  local udid bundle="${IRIS_CHAT_IOS_BUNDLE_ID:-fi.siriusbusiness.irischat}" output pid
  udid="$(ios_udid)"
  [[ -n "$udid" ]] || { echo "No configured iOS simulator found" >&2; exit 1; }
  if [[ "${IRIS_CHAT_IDLE_CPU_IOS_EXCLUSIVE_SIMULATOR:-1}" == 1 ]]; then
    xcrun simctl shutdown all >/dev/null 2>&1 || true
  fi
  if ! ios_simulator_is_booted "$udid" \
    && ! xcrun simctl boot "$udid" >/dev/null 2>&1; then
    if [[ -n "${IRIS_IOS_UDID:-}" ]]; then
      echo "Configured iOS simulator could not boot: $udid" >&2
      exit 1
    fi
    local candidate
    while IFS= read -r candidate; do
      [[ -n "$candidate" && "$candidate" != "$udid" ]] || continue
      if xcrun simctl boot "$candidate" >/dev/null 2>&1; then
        echo "Using bootable iOS simulator $candidate because $udid is unavailable" >&2
        udid="$candidate"
        break
      fi
    done < <(xcrun simctl list -j devices available | python3 -c '
import json, sys
data = json.load(sys.stdin)
for devices in data.get("devices", {}).values():
    for device in devices:
        if device.get("isAvailable", True) and "iPhone" in device.get("name", ""):
            print(device.get("udid", ""))
')
    if ! ios_simulator_is_booted "$udid"; then
      echo "No bootable iOS simulator found" >&2
      exit 1
    fi
  fi
  local create_cmd=(python3 "$ROOT/scripts/run_ios_harness.py" --udid "$udid" --run-id "$RUN_ID" \
    --action create_account_and_report_identity --reset)
  [[ "$SKIP_BUILD" -eq 1 ]] || create_cmd+=(--rebuild)
  "${create_cmd[@]}"
  run_ios_harness "$udid" create_chat_from_args peer_input "$PEER_HEX"
  run_ios_harness "$udid" create_group_from_args group_name "Idle CPU group"
  write_mobile_fixture
  output="$(env \
    SIMCTL_CHILD_IRIS_UI_TEST_RUN_ID="harness-$RUN_ID" \
    SIMCTL_CHILD_IRIS_UI_TEST_BYPASS_KEYCHAIN=1 \
    SIMCTL_CHILD_IRIS_DISABLE_NOTIFICATIONS=1 \
    xcrun simctl launch --terminate-running-process "$udid" "$bundle")"
  pid="$(printf '%s\n' "$output" | sed -n 's/.*: \([0-9][0-9]*\)$/\1/p' | tail -n 1)"
  [[ -n "$pid" ]] || { echo "Could not determine iOS simulator app pid: $output" >&2; exit 1; }
  APP_PID="$pid"
  run_sampler host-pid --pid "$pid" --label "iOS Iris Chat"
}

case "$PLATFORM" in
  macos) run_macos ;;
  linux) run_linux ;;
  android) run_android ;;
  ios) run_ios ;;
  windows)
    command -v powershell.exe >/dev/null 2>&1 || { echo "Windows gate requires powershell.exe" >&2; exit 1; }
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$ROOT/scripts/idle-cpu-platform-gate-windows.ps1"
    ;;
  *) echo "Unsupported platform: $PLATFORM" >&2; exit 2 ;;
esac

echo "idle CPU platform gate passed: $PLATFORM"
echo "run_dir=$RUN_DIR"
