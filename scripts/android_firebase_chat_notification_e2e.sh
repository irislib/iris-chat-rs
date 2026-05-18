#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_DIR="${ROOT_DIR}/android"
PACKAGE_NAME="${ANDROID_PACKAGE_NAME:-to.iris.chat.debug}"
TEST_PACKAGE_NAME="${ANDROID_TEST_PACKAGE_NAME:-to.iris.chat.test}"
TEST_CLASS="to.iris.chat.push.FirebaseChatNotificationE2eTest"
SERIAL="${1:-${ANDROID_SERIAL:-}}"
MESSAGE="${FCM_E2E_MESSAGE:-firebase-chat-e2e-$(date +%s)}"

if [[ -z "${SERIAL}" ]]; then
  SERIAL="$(adb devices | awk '$2 == "device" { print $1; exit }')"
fi

if [[ -z "${SERIAL}" ]]; then
  echo "No Android device found." >&2
  exit 1
fi

project_id() {
  node -e '
const fs = require("fs");
const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
process.stdout.write(config.project_info.project_id);
' "${ROOT_DIR}/android/app/google-services.json"
}

firebase_access_token() {
  if [[ -n "${FIREBASE_ACCESS_TOKEN:-}" ]]; then
    printf '%s' "${FIREBASE_ACCESS_TOKEN}"
    return 0
  fi
  if [[ -n "${FCM_ACCESS_TOKEN_COMMAND:-}" ]]; then
    bash -lc "${FCM_ACCESS_TOKEN_COMMAND}"
    return 0
  fi
  if command -v gcloud >/dev/null 2>&1; then
    gcloud auth application-default print-access-token 2>/dev/null ||
      gcloud auth print-access-token
    return 0
  fi
  return 1
}

run_instrumentation() {
  local method="$1"
  shift
  local output
  output="$(adb -s "${SERIAL}" shell am instrument -w -r \
    -e class "${TEST_CLASS}#${method}" \
    "$@" \
    "${TEST_PACKAGE_NAME}/androidx.test.runner.AndroidJUnitRunner")"
  printf '%s\n' "${output}"
  if printf '%s\n' "${output}" | grep -Eq '(^FAILURES!!!|^INSTRUMENTATION_STATUS_CODE: -[0-9]|^Error in )'; then
    return 1
  fi
}

status_value() {
  local key="$1"
  sed -n "s/^INSTRUMENTATION_STATUS: ${key}=//p" | tail -n 1
}

package_pids() {
  adb -s "${SERIAL}" shell pidof "${PACKAGE_NAME}" 2>/dev/null | tr -d '\r' | xargs || true
}

kill_background_app_process() {
  adb -s "${SERIAL}" shell input keyevent HOME >/dev/null 2>&1 || true
  adb -s "${SERIAL}" shell am kill "${PACKAGE_NAME}" >/dev/null 2>&1 || true
  local pids
  pids="$(package_pids)"
  if [[ -n "${pids}" ]]; then
    adb -s "${SERIAL}" shell run-as "${PACKAGE_NAME}" sh -c "kill -9 ${pids}" >/dev/null 2>&1 ||
      adb -s "${SERIAL}" shell kill -9 ${pids} >/dev/null 2>&1 ||
      true
  fi
}

send_fcm_message() {
  local token="$1"
  local message="$2"
  local project="$3"
  local access_token="$4"
  local request_file response_file http_code
  request_file="$(mktemp)"
  response_file="$(mktemp)"
  node - "${token}" "${message}" >"${request_file}" <<'NODE'
const [token, message] = process.argv.slice(2);
const inner = JSON.stringify({ kind: 14, content: message });
const body = {
  message: {
    token,
    android: {
      priority: "high",
    },
    data: {
      title: "Firebase E2E",
      body: message,
      inner_event_json: inner,
    },
  },
};
process.stdout.write(JSON.stringify(body));
NODE

  http_code="$(
    curl -sS -o "${response_file}" -w '%{http_code}' \
      -H "authorization: Bearer ${access_token}" \
      -H "content-type: application/json; charset=utf-8" \
      -X POST \
      "https://fcm.googleapis.com/v1/projects/${project}/messages:send" \
      --data-binary "@${request_file}"
  )"
  rm -f "${request_file}"
  if [[ "${http_code}" != 2* ]]; then
    echo "Firebase send failed with HTTP ${http_code}:" >&2
    cat "${response_file}" >&2
    rm -f "${response_file}"
    return 1
  fi
  cat "${response_file}"
  rm -f "${response_file}"
}

echo "Building Android debug app and test APK"
IRIS_APP_VERSION_CODE="${IRIS_APP_VERSION_CODE:-900001}" \
IRIS_DEBUG_APPLICATION_ID_SUFFIX="${IRIS_DEBUG_APPLICATION_ID_SUFFIX-.debug}" \
  "${ANDROID_DIR}/gradlew" -p "${ANDROID_DIR}" :app:assembleDebug :app:assembleDebugAndroidTest

echo "Installing on ${SERIAL}"
adb -s "${SERIAL}" install -r -d "${ANDROID_DIR}/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
adb -s "${SERIAL}" install -r -d "${ANDROID_DIR}/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk" >/dev/null
adb -s "${SERIAL}" shell pm grant "${PACKAGE_NAME}" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true
adb -s "${SERIAL}" shell appops set "${PACKAGE_NAME}" POST_NOTIFICATION allow >/dev/null 2>&1 || true

echo "Clearing previous push probe state"
run_instrumentation clear_push_probe >/dev/null

echo "Requesting FCM registration token"
token_output="$(run_instrumentation report_fcm_token)"
fcm_token="$(printf '%s\n' "${token_output}" | status_value fcm_token)"
if [[ -z "${fcm_token}" ]]; then
  echo "${token_output}" >&2
  echo "FCM token was not reported by instrumentation." >&2
  exit 1
fi

project="$(project_id)"
access_token="$(firebase_access_token | tr -d '\r\n')"
if [[ -z "${access_token}" ]]; then
  echo "No Firebase access token available. Set FIREBASE_ACCESS_TOKEN or FCM_ACCESS_TOKEN_COMMAND." >&2
  exit 1
fi

# Verifies that an FCM data push wakes the app when no Iris process is
# running. Do not use `am force-stop` here: Android intentionally blocks
# a force-stopped package from receiving push until the next explicit
# launch, which is a different state from the user swiping the app away
# or the system killing the background process.
echo "Closing ${PACKAGE_NAME} on ${SERIAL} before sending FCM"
kill_background_app_process
remaining_pids="$(package_pids)"
if [[ -n "${remaining_pids}" ]]; then
  echo "Warning: ${PACKAGE_NAME} still has process id(s) before FCM send: ${remaining_pids}" >&2
fi

if ! send_fcm_message "${fcm_token}" "${MESSAGE}" "${project}" "${access_token}" >/dev/null; then
  exit 1
fi

echo "Waiting for Firebase to wake the closed app process"
woke_pids=""
for _ in $(seq 1 "${FCM_E2E_WAKE_POLL_SECS:-45}"); do
  woke_pids="$(package_pids)"
  if [[ -n "${woke_pids}" ]]; then
    break
  fi
  sleep 1
done
if [[ -z "${woke_pids}" ]]; then
  echo "Warning: no ${PACKAGE_NAME} process observed before verification; checking notification probe anyway." >&2
else
  echo "Firebase woke ${PACKAGE_NAME} with process id(s): ${woke_pids}"
  sleep "${FCM_E2E_POST_WAKE_GRACE_SECS:-10}"
fi

echo "Verifying Android notification probe and active notification"
wait_output="$(mktemp)"
if ! adb -s "${SERIAL}" shell am instrument -w -r \
  -e class "${TEST_CLASS}#wait_for_firebase_chat_notification" \
  -e message "${MESSAGE}" \
  -e timeout_ms "${FCM_E2E_TIMEOUT_MS:-120000}" \
  "${TEST_PACKAGE_NAME}/androidx.test.runner.AndroidJUnitRunner" >"${wait_output}" 2>&1 ||
  grep -Eq '(^FAILURES!!!|^INSTRUMENTATION_STATUS_CODE: -[0-9]|^Error in )' "${wait_output}"; then
  cat "${wait_output}" >&2
  rm -f "${wait_output}"
  exit 1
fi
cat "${wait_output}"
rm -f "${wait_output}"

echo "Android Firebase chat notification e2e passed for ${SERIAL}"
