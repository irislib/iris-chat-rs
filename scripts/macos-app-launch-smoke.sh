#!/usr/bin/env bash

set -Eeuo pipefail

APP_PATH="${IRIS_MACOS_APP_SMOKE_PATH:-${1:-}}"
ALIVE_SECONDS="${IRIS_MACOS_APP_SMOKE_ALIVE_SECONDS:-5}"
STARTUP_TIMEOUT_SECONDS="${IRIS_MACOS_APP_SMOKE_STARTUP_TIMEOUT_SECONDS:-15}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS app launch smoke requires macOS." >&2
  exit 1
fi
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "macOS app bundle not found: ${APP_PATH:-<unset>}" >&2
  exit 1
fi
if [[ ! "$ALIVE_SECONDS" =~ ^[0-9]+$ || "$ALIVE_SECONDS" -lt 1 ]]; then
  echo "IRIS_MACOS_APP_SMOKE_ALIVE_SECONDS must be a positive integer." >&2
  exit 2
fi
if [[ ! "$STARTUP_TIMEOUT_SECONDS" =~ ^[0-9]+$ || "$STARTUP_TIMEOUT_SECONDS" -lt 1 ]]; then
  echo "IRIS_MACOS_APP_SMOKE_STARTUP_TIMEOUT_SECONDS must be a positive integer." >&2
  exit 2
fi

executable_name="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP_PATH/Contents/Info.plist"
)"
executable="$APP_PATH/Contents/MacOS/$executable_name"
if [[ ! -x "$executable" ]]; then
  echo "macOS app executable not found: $executable" >&2
  exit 1
fi

run_id="release-smoke-$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="$(mktemp -d "${TMPDIR:-/tmp}/iris-macos-release-smoke.XXXXXX")"
data_dir="$run_dir/data"
log_path="$run_dir/app.log"
pid=""
mkdir -p "$data_dir"

cleanup() {
  if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" 2>/dev/null || true
  fi
  if [[ "${IRIS_MACOS_APP_SMOKE_KEEP_ARTIFACTS:-0}" != "1" ]]; then
    rm -rf "$run_dir"
  else
    echo "macOS launch smoke artifacts: $run_dir"
  fi
}
trap cleanup EXIT

env \
  IRIS_UI_TEST_RESET=1 \
  IRIS_UI_TEST_RUN_ID="$run_id" \
  IRIS_UI_TEST_DATA_DIR="$data_dir" \
  IRIS_UI_TEST_BYPASS_KEYCHAIN=1 \
  IRIS_DISABLE_NOTIFICATIONS_FOR_AUTOMATION=1 \
  "$executable" >"$log_path" 2>&1 &
pid=$!

deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
while (( SECONDS < deadline )); do
  if kill -0 "$pid" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

if ! kill -0 "$pid" >/dev/null 2>&1; then
  echo "macOS app exited during startup." >&2
  tail -n 100 "$log_path" >&2 || true
  exit 1
fi

alive_until=$((SECONDS + ALIVE_SECONDS))
while (( SECONDS < alive_until )); do
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    echo "macOS app exited before the ${ALIVE_SECONDS}s smoke window completed." >&2
    tail -n 100 "$log_path" >&2 || true
    exit 1
  fi
  sleep 0.25
done

echo "MACOS_RELEASE_APP_SMOKE_OK"
