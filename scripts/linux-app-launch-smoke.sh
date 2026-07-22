#!/usr/bin/env bash

set -Eeuo pipefail

APP_PATH="${IRIS_LINUX_APP_SMOKE_PATH:-${1:-}}"
ALIVE_SECONDS="${IRIS_LINUX_APP_SMOKE_ALIVE_SECONDS:-5}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Linux app launch smoke requires Linux." >&2
  exit 1
fi
if [[ -z "$APP_PATH" || ! -x "$APP_PATH" ]]; then
  echo "Linux app executable not found: ${APP_PATH:-<unset>}" >&2
  exit 1
fi
if [[ ! "$ALIVE_SECONDS" =~ ^[0-9]+$ || "$ALIVE_SECONDS" -lt 1 ]]; then
  echo "IRIS_LINUX_APP_SMOKE_ALIVE_SECONDS must be a positive integer." >&2
  exit 2
fi

run_id="release-smoke-$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="$(mktemp -d "${TMPDIR:-/tmp}/iris-linux-release-smoke.XXXXXX")"
data_dir="$run_dir/data"
runtime_dir="$run_dir/runtime"
log_path="$run_dir/app.log"
app_pid=""
display_pid=""
mkdir -p "$data_dir" "$runtime_dir" "$run_dir/home"
chmod 700 "$runtime_dir"

cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" >/dev/null 2>&1; then
    kill -- "-$app_pid" >/dev/null 2>&1 || kill "$app_pid" >/dev/null 2>&1 || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if [[ -n "$display_pid" ]] && kill -0 "$display_pid" >/dev/null 2>&1; then
    kill "$display_pid" >/dev/null 2>&1 || true
    wait "$display_pid" 2>/dev/null || true
  fi
  if [[ "${IRIS_LINUX_APP_SMOKE_KEEP_ARTIFACTS:-0}" != "1" ]]; then
    rm -rf "$run_dir"
  else
    echo "Linux launch smoke artifacts: $run_dir"
  fi
}
trap cleanup EXIT

if ! xdpyinfo -display "${DISPLAY:-:invalid}" >/dev/null 2>&1; then
  display_number="${DISPLAY:-${IRIS_LINUX_APP_SMOKE_DISPLAY:-:97}}"
  Xvfb "$display_number" -screen 0 1280x800x24 -nolisten tcp >"$run_dir/xvfb.log" 2>&1 &
  display_pid=$!
  export DISPLAY="$display_number"
  for _ in {1..40}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
      break
    fi
    sleep 0.25
  done
  xdpyinfo -display "$DISPLAY" >/dev/null
fi

setsid dbus-run-session -- env \
  HOME="$run_dir/home" \
  XDG_RUNTIME_DIR="$runtime_dir" \
  IRIS_UI_TEST_RESET=1 \
  IRIS_UI_TEST_RUN_ID="$run_id" \
  IRIS_UI_TEST_DATA_DIR="$data_dir" \
  "$APP_PATH" >"$log_path" 2>&1 &
app_pid=$!

alive_until=$((SECONDS + ALIVE_SECONDS))
while (( SECONDS < alive_until )); do
  if ! kill -0 "$app_pid" >/dev/null 2>&1; then
    echo "Linux app exited before the ${ALIVE_SECONDS}s smoke window completed." >&2
    tail -n 100 "$log_path" >&2 || true
    exit 1
  fi
  sleep 0.25
done

echo "LINUX_RELEASE_APP_SMOKE_OK"
