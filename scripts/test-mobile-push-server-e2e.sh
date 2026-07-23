#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

cat >"$TMP_DIR/adb" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "${1:-}" == "devices" ]]; then
  printf 'List of devices attached\nphysical-device\tdevice\nemulator-5554\tdevice\n'
elif [[ "${1:-}" == "-s" && "${2:-}" == emulator-* ]]; then
  cat >/dev/null
  printf 'package:/product/priv-app/PrebuiltGmsCore/PrebuiltGmsCore.apk\n'
elif [[ "${1:-}" == "-s" ]]; then
  cat >/dev/null
fi
EOF
chmod +x "$TMP_DIR/adb"
cat >"$TMP_DIR/run-emulators" <<'EOF'
#!/usr/bin/env bash
printf 'Iris_Android_E2E_B emulator-5556\n'
EOF
chmod +x "$TMP_DIR/run-emulators"

IRIS_MOBILE_PUSH_E2E_SOURCE_ONLY=1 source "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- '-collect-test-diagnostics never' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- '-maximum-test-execution-time-allowance 90' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'arm_and_wait_for_mobile_push_delivery' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'HARNESS_STATUS: probe_id=' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'IOS_PUSH_PID=\$!' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'MobilePushDeliveryProbe.recordIfArmed()' "$ROOT/ios/Sources/MobilePushSupport.swift"
grep -q -- 'current_apns_author' "$ROOT/ios/Tests/InteropHarnessReportingHelpers.swift"
grep -q -- 'current_fcm_author=\[1-9\]' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'FCM_E2E_POST_WAKE_GRACE_SECS:-10' "$ROOT/scripts/mobile_push_server_e2e.sh"
grep -q -- 'activeNotificationSnapshots().firstOrNull' \
  "$ROOT/android/app/src/androidTest/java/to/iris/chat/push/FirebaseChatNotificationE2eTest.kt"
printf 'INSTRUMENTATION_STATUS: notification_authorization=UNAuthorizationStatus(rawValue: 2)\n' |
  ios_notifications_deliverable
! printf 'INSTRUMENTATION_STATUS: notification_authorization=UNAuthorizationStatus(rawValue: 1)\n' |
  ios_notifications_deliverable
ADB="$TMP_DIR/adb"
ANDROID_EMULATOR_RUNNER="$TMP_DIR/run-emulators"

[[ "$(resolve_android_serial)" == "emulator-5554" ]]
[[ "$(resolve_android_push_serial)" == "emulator-5556" ]]
has_google_play_services emulator-5554
! has_google_play_services physical-device

echo "Mobile push FCM device selection harness passed."
