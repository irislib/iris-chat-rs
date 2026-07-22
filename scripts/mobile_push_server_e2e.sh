#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_URL="${IRIS_MOBILE_PUSH_E2E_SERVER_URL:-https://notifications.iris.to}"
IOS_HARNESS="$ROOT/scripts/run_ios_harness.py"
ANDROID_HARNESS="$ROOT/scripts/run_harness.py"
ANDROID_PACKAGE="to.iris.chat.debug"
ANDROID_TEST_PACKAGE="to.iris.chat.test"
ANDROID_RUNNER="$ANDROID_TEST_PACKAGE/androidx.test.runner.AndroidJUnitRunner"
ANDROID_CHAT_CLASS="to.iris.chat.RealRelayHarnessTest"
ANDROID_PUSH_CLASS="to.iris.chat.push.FirebaseChatNotificationE2eTest"
ANDROID_EMULATOR_RUNNER="${IRIS_ANDROID_EMULATOR_RUNNER:-$ROOT/scripts/run_android_emulators.sh}"
RUN_ID="mobile-push-$(date +%s)"
IOS_RUN_ID="$RUN_ID-ios"
MESSAGE_IOS="server-apns-$RUN_ID"
MESSAGE_ANDROID="server-fcm-$RUN_ID"

android_sdk() {
  if [[ -n "${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}" ]]; then
    printf '%s\n' "${ANDROID_HOME:-$ANDROID_SDK_ROOT}"
    return
  fi
  sed -n 's/^sdk\.dir=//p' "$ROOT/android/local.properties" 2>/dev/null | tail -n 1
}

resolve_ios_udid() {
  if [[ -n "${IRIS_IOS_PUSH_E2E_UDID:-}" ]]; then
    printf '%s\n' "$IRIS_IOS_PUSH_E2E_UDID"
    return
  fi
  xcrun xctrace list devices 2>/dev/null |
    sed -n '/^== Devices ==$/,/^== Devices Offline ==$/p' |
    awk '/iPhone/ { print $NF; exit }' |
    tr -d '()'
}

resolve_android_serial() {
  if [[ -n "${IRIS_ANDROID_PUSH_E2E_SERIAL:-${ANDROID_SERIAL:-}}" ]]; then
    printf '%s\n' "${IRIS_ANDROID_PUSH_E2E_SERIAL:-$ANDROID_SERIAL}"
    return
  fi
  local serial status
  while read -r serial status; do
    if [[ "$status" == "device" ]] && has_google_play_services "$serial"; then
      printf '%s\n' "$serial"
      return
    fi
  done < <("$ADB" devices | tail -n +2)
}

resolve_android_push_serial() {
  if [[ -n "${IRIS_ANDROID_PUSH_E2E_SERIAL:-${ANDROID_SERIAL:-}}" ]]; then
    resolve_android_serial
    return
  fi

  local avd="${IRIS_ANDROID_PUSH_E2E_AVD:-Iris_Android_E2E_B}"
  if [[ "$avd" != "off" ]]; then
    local boot_output serial
    boot_output="$("$ANDROID_EMULATOR_RUNNER" --headless "$avd")"
    serial="$(awk -v avd="$avd" '$1 == avd { print $2; exit }' <<<"$boot_output")"
    if [[ -n "$serial" ]] && has_google_play_services "$serial"; then
      printf '%s\n' "$serial"
      return
    fi
  fi

  resolve_android_serial
}

has_google_play_services() {
  local packages
  packages="$("$ADB" -s "$1" shell pm path com.google.android.gms </dev/null 2>/dev/null || true)"
  [[ "$packages" == *"package:"* ]]
}

status_value() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

require_value() {
  local label="$1" value="$2"
  if [[ -z "$value" ]]; then
    echo "Missing ${label} from E2E harness output." >&2
    exit 1
  fi
}

run_ios() {
  local action="$1"
  shift
  local cmd=(
    python3 "$IOS_HARNESS"
    --udid "$IOS_UDID"
    --run-id "$IOS_RUN_ID"
    --action "$action"
    --enable-notifications
    --notification-server-url "$SERVER_URL"
    --timeout-secs 480
  )
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --reset|--rebuild) cmd+=("$1"); shift ;;
      *) cmd+=(--arg "$1=$2"); shift 2 ;;
    esac
  done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "$output" >&2
    return 1
  }
  printf '%s\n' "$output"
  printf '%s\n' "$output" | grep -q '^INSTRUMENTATION_CODE: -1$'
}

run_android() {
  local class_name="$1" test_name="$2"
  shift 2
  local cmd=(
    python3 "$ANDROID_HARNESS"
    --adb "$ADB"
    --serial "$ANDROID_SERIAL_RESOLVED"
    --runner "$ANDROID_RUNNER"
    --class-name "$class_name"
    --test-name "$test_name"
    --arg clearPackageData=false
  )
  while [[ $# -gt 0 ]]; do
    cmd+=(--arg "$1=$2")
    shift 2
  done
  local output
  output="$("${cmd[@]}" 2>&1)" || {
    printf '%s\n' "$output" >&2
    return 1
  }
  printf '%s\n' "$output"
  ! printf '%s\n' "$output" |
    grep -Eq '^(FAILURES!!!|INSTRUMENTATION_STATUS_CODE: -[0-9]|INSTRUMENTATION_FAILED:|Error in )'
}

grant_ios_notification_permission() {
  local xctestrun
  xctestrun="$(find "$ROOT/ios/.build/harness-derived-data/Build/Products" \
    -maxdepth 2 -name '*.xctestrun' -path '*iphoneos*' -print | head -n 1)"
  require_value "physical-device xctestrun" "$xctestrun"
  local command=(
    xcodebuild
    test-without-building
    -xctestrun "$xctestrun"
    -destination "id=$IOS_UDID"
    -parallel-testing-enabled NO
    -only-testing:IrisChatUITests/IrisChatUITests/testGrantNotificationPermissionForProductionPushE2E
  )
  "${command[@]}" || {
    echo "Retrying transient iOS UI automation startup failure" >&2
    "${command[@]}"
  }
}

cleanup() {
  set +e
  run_ios clear_mobile_push_delivery_probe >/dev/null 2>&1
  run_ios disable_mobile_push_and_wait >/dev/null 2>&1
  run_android "$ANDROID_CHAT_CLASS" disable_mobile_push_and_wait >/dev/null 2>&1
}

if [[ "${IRIS_MOBILE_PUSH_E2E_SOURCE_ONLY:-0}" == "1" ]]; then
  return 0 2>/dev/null || exit 0
fi
trap cleanup EXIT

SDK="$(android_sdk)"
require_value "Android SDK" "$SDK"
ADB="$SDK/platform-tools/adb"
IOS_UDID="$(resolve_ios_udid)"
ANDROID_SERIAL_RESOLVED="$(resolve_android_push_serial)"
require_value "connected physical iPhone" "$IOS_UDID"
require_value "connected FCM-capable Android device" "$ANDROID_SERIAL_RESOLVED"
if xcrun simctl list devices 2>/dev/null | grep -q "$IOS_UDID"; then
  echo "The iOS notification E2E requires a physical iPhone, not a simulator." >&2
  exit 1
fi
if ! has_google_play_services "$ANDROID_SERIAL_RESOLVED"; then
  echo "The Android notification E2E requires Google Play services for FCM." >&2
  exit 1
fi

echo "Building physical iOS test host with current Rust core"
"$ROOT/scripts/ios-build" ios-xcframework
"$ROOT/scripts/ios-build" ios-xcodeproj
IOS_IDENTITY="$(run_ios create_account_and_report_identity --reset --rebuild display_name 'iOS push E2E')"
IOS_NPUB="$(printf '%s\n' "$IOS_IDENTITY" | status_value npub)"
require_value "iOS user ID" "$IOS_NPUB"

echo "Granting and verifying iOS notification permission"
grant_ios_notification_permission >/dev/null
run_ios report_notification_authorization >/dev/null

echo "Building and installing Android E2E packages"
(
  cd "$ROOT/android"
  IRIS_MOBILE_PUSH_SERVER_URL="$SERVER_URL" \
    ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest
)
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" install -r -d \
  "$ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" install -r -d \
  "$ROOT/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk" >/dev/null
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" shell pm clear "$ANDROID_PACKAGE" >/dev/null
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" shell pm grant \
  "$ANDROID_PACKAGE" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true

ANDROID_IDENTITY="$(run_android "$ANDROID_CHAT_CLASS" create_account_and_report_identity)"
ANDROID_NPUB="$(printf '%s\n' "$ANDROID_IDENTITY" | status_value npub)"
require_value "Android user ID" "$ANDROID_NPUB"

echo "Establishing an encrypted chat and both push subscriptions"
run_ios create_chat_from_args peer_input "$ANDROID_NPUB" >/dev/null
run_ios send_message_from_args peer_input "$ANDROID_NPUB" message "seed-$RUN_ID" >/dev/null
run_android "$ANDROID_CHAT_CLASS" wait_for_message_from_args \
  peer_input "$IOS_NPUB" message "seed-$RUN_ID" direction incoming timeout_secs 180 >/dev/null
run_android "$ANDROID_CHAT_CLASS" send_message_from_args \
  peer_input "$IOS_NPUB" message "ready-$RUN_ID" >/dev/null
run_ios wait_for_message_from_args \
  peer_input "$ANDROID_NPUB" message "ready-$RUN_ID" direction incoming timeout_secs 180 >/dev/null

IOS_SUBSCRIPTIONS=""
for _ in $(seq 1 12); do
  IOS_SUBSCRIPTIONS="$(run_ios report_mobile_push_server_snapshot 2>/dev/null || true)"
  if printf '%s\n' "$IOS_SUBSCRIPTIONS" | grep -Eq 'subscriptions=.*apns=[1-9]'; then
    break
  fi
  sleep 2
done
if ! printf '%s\n' "$IOS_SUBSCRIPTIONS" | grep -Eq 'subscriptions=.*apns=[1-9]'; then
  echo "notifications.iris.to did not report an iOS APNs subscription." >&2
  printf '%s\n' "$IOS_SUBSCRIPTIONS" >&2
  exit 1
fi

ANDROID_SUBSCRIPTIONS=""
for _ in $(seq 1 12); do
  ANDROID_SUBSCRIPTIONS="$(
    run_android "$ANDROID_CHAT_CLASS" report_mobile_push_server_snapshot 2>/dev/null || true
  )"
  if printf '%s\n' "$ANDROID_SUBSCRIPTIONS" | grep -Eq 'subscriptions=.*fcm=[1-9]'; then
    break
  fi
  sleep 2
done
if ! printf '%s\n' "$ANDROID_SUBSCRIPTIONS" | grep -Eq 'subscriptions=.*fcm=[1-9]'; then
  echo "notifications.iris.to did not report an Android FCM subscription." >&2
  printf '%s\n' "$ANDROID_SUBSCRIPTIONS" >&2
  exit 1
fi

echo "Verifying notifications.iris.to → APNs → physical iPhone"
run_ios clear_delivered_notifications >/dev/null
IOS_PROBE="$(run_ios arm_mobile_push_delivery_probe)"
IOS_PROBE_ID="$(printf '%s\n' "$IOS_PROBE" | status_value probe_id)"
require_value "iOS delivery probe" "$IOS_PROBE_ID"
run_android "$ANDROID_CHAT_CLASS" send_message_from_args \
  peer_input "$IOS_NPUB" message "$MESSAGE_IOS" >/dev/null
IOS_PUSH="$(run_ios wait_for_mobile_push_delivery_probe probe_id "$IOS_PROBE_ID" timeout_secs 90)"
require_value "iOS delivered notification" "$(printf '%s\n' "$IOS_PUSH" | status_value probe_id)"

echo "Verifying notifications.iris.to → FCM → physical Android"
run_android "$ANDROID_PUSH_CLASS" clear_push_probe >/dev/null
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" shell input keyevent HOME >/dev/null 2>&1 || true
"$ADB" -s "$ANDROID_SERIAL_RESOLVED" shell am kill "$ANDROID_PACKAGE" >/dev/null 2>&1 || true
run_ios send_message_from_args \
  peer_input "$ANDROID_NPUB" message "$MESSAGE_ANDROID" wait_for_delivery false >/dev/null
run_android "$ANDROID_PUSH_CLASS" wait_for_firebase_chat_notification \
  message "$MESSAGE_ANDROID" timeout_ms 120000 >/dev/null

echo "Mobile push server E2E passed for physical iOS (APNs) and Android (FCM)."
