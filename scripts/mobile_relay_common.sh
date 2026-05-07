#!/usr/bin/env bash

local_relay_host() {
  printf '%s' "${IRIS_LOCAL_RELAY_HOST:-127.0.0.1}"
}

local_relay_port() {
  printf '%s' "${IRIS_LOCAL_RELAY_PORT:-4848}"
}

local_relay_set_id() {
  printf '%s' "${IRIS_LOCAL_RELAY_SET_ID:-local-interop}"
}

local_android_relay_url() {
  printf 'ws://10.0.2.2:%s' "$(local_relay_port)"
}

local_android_loopback_relay_url() {
  printf 'ws://127.0.0.1:%s' "$(local_relay_port)"
}

local_ios_relay_url() {
  printf 'ws://127.0.0.1:%s' "$(local_relay_port)"
}

local_relay_runner() {
  printf '%s/scripts/local_nostr_relay.py' "${ROOT_DIR}"
}

websocket_healthcheck() {
  local host="$1"
  local port="$2"
  local websocket_key='bmRyLWRlbW8taGVhbHRoLWNoZWNr'
  local line

  if ! exec 3<>"/dev/tcp/${host}/${port}" 2>/dev/null; then
    return 1
  fi

  printf 'GET / HTTP/1.1\r\nHost: %s:%s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n' \
    "${host}" "${port}" "${websocket_key}" >&3

  IFS= read -r -t 2 line <&3 || {
    exec 3<&-
    exec 3>&-
    return 1
  }

  exec 3<&-
  exec 3>&-
  [[ "${line}" == *"101"* ]]
}

assert_local_relay_healthy() {
  local host="${1:-$(local_relay_host)}"
  local port="${2:-$(local_relay_port)}"
  if ! websocket_healthcheck "${host}" "${port}"; then
    echo "Local relay is not accepting websocket connections on ${host}:${port}." >&2
    echo "Start it with: $(local_relay_runner)" >&2
    return 1
  fi
}

wait_for_local_relay_healthy() {
  local host="${1:-$(local_relay_host)}"
  local port="${2:-$(local_relay_port)}"
  local timeout_secs="${3:-15}"
  local deadline=$((SECONDS + timeout_secs))
  while (( SECONDS < deadline )); do
    if websocket_healthcheck "${host}" "${port}"; then
      return 0
    fi
    sleep 1
  done
  echo "Timed out waiting for local relay on ${host}:${port}." >&2
  return 1
}

start_local_rust_relay() {
  local log_file="${1:-/tmp/iris-chat-local-relay.log}"
  local bind_addr="0.0.0.0:$(local_relay_port)"
  local pid

  "$(local_relay_runner)" "${bind_addr}" >"${log_file}" 2>&1 &
  pid=$!
  if ! wait_for_local_relay_healthy "$(local_relay_host)" "$(local_relay_port)" 90; then
    if kill -0 "${pid}" >/dev/null 2>&1; then
      kill "${pid}" >/dev/null 2>&1 || true
      wait "${pid}" >/dev/null 2>&1 || true
    fi
    echo "Relay log: ${log_file}" >&2
    return 1
  fi

  printf '%s\n' "${pid}"
}

stop_local_rust_relay() {
  local pid="$1"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" >/dev/null 2>&1; then
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" >/dev/null 2>&1 || true
  fi
}

android_debug_apk() {
  printf '%s/android/app/build/outputs/apk/debug/app-debug.apk' "${ROOT_DIR}"
}

android_debug_test_apk() {
  printf '%s/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk' "${ROOT_DIR}"
}

build_android_debug_apks() {
  local relay_url="$1"
  local relay_set_id="$2"

  (
    cd "${ROOT_DIR}/android" &&
      IRIS_DEBUG_RELAYS="${relay_url}" \
      IRIS_DEBUG_RELAY_SET_ID="${relay_set_id}" \
      ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest
  )
}

install_android_debug_apks_on_serials() {
  local adb="$1"
  shift

  local serial
  for serial in "$@"; do
    [[ -n "${serial}" ]] || continue
    echo "Installing Android debug APKs on ${serial}"
    "${adb}" -s "${serial}" install -r -d "$(android_debug_apk)" >/dev/null
    "${adb}" -s "${serial}" install -r -d -t "$(android_debug_test_apk)" >/dev/null
  done
}
