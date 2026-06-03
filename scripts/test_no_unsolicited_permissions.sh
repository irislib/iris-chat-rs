#!/usr/bin/env bash
#
# Verify the iOS app doesn't trigger Bluetooth or Local Network
# permission prompts before the user opens the Nearby feature.
#
# `simctl privacy` doesn't expose grant/revoke for Bluetooth or Local
# Network, so this script wipes the target simulator entirely with
# `simctl erase` before running the test — that's the only way to
# restore the "permission not determined" state programmatically.
#
# Usage:
#   ./scripts/test_no_unsolicited_permissions.sh                # default sim
#   ./scripts/test_no_unsolicited_permissions.sh "iPhone 17 Pro"

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/e2e_prerelease_common.sh"
IOS_DIR="$ROOT/ios"
PROJECT="$IOS_DIR/IrisChat.xcodeproj"
SCHEME="IrisChat"
TEST_ID="IrisChatUITests/IrisChatUITests/testNoUnsolicitedPermissionPromptsOnFirstLaunch"
DERIVED_DATA="$IOS_DIR/.build/permissions-test"
DEVICE="${1:-iPhone 17 Pro}"

export IRIS_APP_VERSION_CODE="${IRIS_APP_VERSION_CODE:-0}"
export IRIS_XCODE_MARKETING_VERSION="${IRIS_XCODE_MARKETING_VERSION:-0.0.0}"

resolve_udid() {
  local name="$1"
  xcrun simctl list -j devices available |
    /usr/bin/python3 -c "
import json, sys
name = sys.argv[1]
data = json.load(sys.stdin)
for runtime, devices in sorted(data.get('devices', {}).items(), reverse=True):
    for d in devices:
        if d.get('name') == name and d.get('isAvailable', True):
            print(d.get('udid'))
            sys.exit(0)
sys.exit(1)
" "$name"
}

udid="$(resolve_udid "$DEVICE" || true)"
if [[ -z "${udid:-}" ]]; then
  echo "Simulator not found: $DEVICE" >&2
  exit 1
fi

cleanup() {
  local exit_code=$?
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" != "1" ]]; then
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  fi
  exit "${exit_code}"
}
trap cleanup EXIT

echo "▶︎  Shutting down + erasing $DEVICE ($udid) to clear stale permission grants"
xcrun simctl shutdown all >/dev/null 2>&1 || true
# Restart CoreSimulator: after a freshly-erased sim, the install
# service in the existing daemon sometimes refuses with "Failed to
# create promise". Killing the daemon and re-booting resets the
# install-service state too.
killall -9 com.apple.CoreSimulator.CoreSimulatorService 2>/dev/null || true
sleep 2
xcrun simctl erase "$udid"
xcrun simctl boot "$udid"
# Apps installed too early after a fresh erase + boot get rejected with
# "Simulator device failed to install the application" because the
# install service isn't ready. Wait for SpringBoard to be up.
iris_e2e_wait_for_ios_bootstatus "$udid"
sleep 3

echo "▶︎  Building tests"
xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -destination "id=$udid" \
  -derivedDataPath "$DERIVED_DATA" \
  -only-testing "$TEST_ID" \
  ONLY_ACTIVE_ARCH=YES \
  build-for-testing \
  -quiet

echo "▶︎  Running permission-prompt test on $DEVICE"
xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -destination "id=$udid" \
  -derivedDataPath "$DERIVED_DATA" \
  -only-testing "$TEST_ID" \
  test-without-building \
  -quiet
