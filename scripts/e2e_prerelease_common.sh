#!/usr/bin/env bash

set -Eeuo pipefail

iris_e2e_default_public_relays() {
  printf '%s' "${IRIS_E2E_RELAYS:-wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to}"
}

iris_e2e_stamp() {
  date -u +%Y%m%dT%H%M%SZ
}

iris_e2e_extract_status() {
  local key="$1"
  sed -n \
    -e "s/^INSTRUMENTATION_STATUS: ${key}=//p" \
    -e "s/^${key}=//p" | tail -n 1
}

iris_e2e_require_value() {
  local name="$1"
  local value="$2"
  if [[ -z "${value}" ]]; then
    echo "Missing required status value: ${name}" >&2
    return 1
  fi
}

iris_e2e_wait_for_status_in_file() {
  local file="$1"
  local key="$2"
  local timeout_secs="$3"
  local deadline=$((SECONDS + timeout_secs))
  local value=""
  while (( SECONDS < deadline )); do
    if [[ -f "${file}" ]]; then
      value="$(iris_e2e_extract_status "${key}" <"${file}")"
      if [[ -n "${value}" ]]; then
        printf '%s\n' "${value}"
        return 0
      fi
    fi
    sleep 1
  done
  echo "Timed out waiting for ${key} in ${file}" >&2
  return 1
}

iris_e2e_record_repo_trace() {
  local root_dir="$1"
  local run_dir="$2"
  {
    printf 'iris_chat_rs_head=%s\n' "$(git -C "${root_dir}" rev-parse HEAD)"
    printf 'iris_chat_rs_branch=%s\n' "$(git -C "${root_dir}" rev-parse --abbrev-ref HEAD)"
    if [[ -d "${root_dir}/../nostr-double-ratchet/.git" ]]; then
      printf 'nostr_double_ratchet_head=%s\n' "$(git -C "${root_dir}/../nostr-double-ratchet" rev-parse HEAD)"
      printf 'nostr_double_ratchet_branch=%s\n' "$(git -C "${root_dir}/../nostr-double-ratchet" rev-parse --abbrev-ref HEAD)"
    fi
  } >"${run_dir}/repo-trace.env"
}

iris_e2e_resolve_android_sdk() {
  local root_dir="$1"
  local local_properties="${root_dir}/android/local.properties"
  local sdk_dir="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "${sdk_dir}" && -f "${local_properties}" ]]; then
    sdk_dir="$(sed -n 's/^sdk\.dir=//p' "${local_properties}" | tail -n 1)"
  fi
  if [[ -z "${sdk_dir}" ]]; then
    echo "Android SDK path not found. Set ANDROID_HOME, ANDROID_SDK_ROOT, or sdk.dir in android/local.properties." >&2
    return 1
  fi
  printf '%s' "${sdk_dir}"
}

iris_e2e_run_and_log() {
  local log_file="$1"
  shift
  {
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  } | tee -a "${log_file}" >&2
  "$@" 2>&1 | tee -a "${log_file}"
}

iris_e2e_shutdown_stale_ios_simulators() {
  if ! command -v xcrun >/dev/null 2>&1; then
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
  iris_e2e_quit_idle_ios_simulator_app
}

iris_e2e_shutdown_ios_simulators() {
  if ! command -v xcrun >/dev/null 2>&1; then
    return 0
  fi

  local udid=""
  for udid in "$@"; do
    [[ -n "${udid}" ]] || continue
    xcrun simctl shutdown "${udid}" >/dev/null 2>&1 || true
  done
  iris_e2e_quit_idle_ios_simulator_app
}

iris_e2e_quit_idle_ios_simulator_app() {
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
    return 0
  fi
  if ! command -v xcrun >/dev/null 2>&1; then
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

iris_e2e_wait_for_ios_bootstatus() {
  local udid="$1"
  local timeout_secs="${2:-${IRIS_IOS_BOOTSTATUS_TIMEOUT_SECS:-120}}"
  local fallback_sleep="${IRIS_IOS_BOOTSTATUS_FALLBACK_SLEEP_SECS:-20}"
  local deadline=$((SECONDS + timeout_secs))
  local pid=""

  xcrun simctl bootstatus "${udid}" -b >/dev/null 2>&1 &
  pid=$!
  while kill -0 "${pid}" >/dev/null 2>&1; do
    if (( SECONDS >= deadline )); then
      kill "${pid}" >/dev/null 2>&1 || true
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

iris_e2e_android_instrumentation_succeeded() {
  local output="$1"
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_CODE: -1$'; then
    return 0
  fi
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_STATUS_CODE: -'; then
    return 1
  fi
  if printf '%s\n' "${output}" | rg -q '^FAILURES!!!$'; then
    return 1
  fi
  if printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_STATUS_CODE: 0$' &&
    printf '%s\n' "${output}" | rg -q '^INSTRUMENTATION_RESULT: shortMsg=Process crashed\.?$'; then
    return 0
  fi
  return 1
}

iris_e2e_android_instrumentation_file_succeeded() {
  local file="$1"
  if [[ ! -f "${file}" ]]; then
    return 1
  fi
  iris_e2e_android_instrumentation_succeeded "$(cat "${file}")"
}

iris_e2e_ensure_android_package() {
  local adb="$1"
  local serial="$2"
  local package_name="$3"
  local apk_path="$4"
  local log_file="$5"

  if "${adb}" -s "${serial}" shell pm path "${package_name}" >/dev/null 2>&1; then
    return 0
  fi

  {
    printf 'Android package %s missing on %s; reinstalling %s\n' \
      "${package_name}" "${serial}" "${apk_path}"
  } | tee -a "${log_file}" >&2
  "${adb}" -s "${serial}" install -r "${apk_path}" 2>&1 | tee -a "${log_file}"
  "${adb}" -s "${serial}" shell pm path "${package_name}" >/dev/null
}

iris_e2e_install_android_package() {
  local adb="$1"
  local serial="$2"
  local package_name="$3"
  local apk_path="$4"
  local log_file="$5"

  {
    printf 'Installing Android package %s on %s from %s\n' \
      "${package_name}" "${serial}" "${apk_path}"
  } | tee -a "${log_file}" >&2
  "${adb}" -s "${serial}" install -r "${apk_path}" 2>&1 | tee -a "${log_file}"
  "${adb}" -s "${serial}" shell pm path "${package_name}" >/dev/null
}

iris_e2e_wait_android_public_network() {
  local adb="$1"
  local serial="$2"
  local timeout_secs="${3:-60}"
  local deadline=$((SECONDS + timeout_secs))
  while (( SECONDS < deadline )); do
    if "${adb}" -s "${serial}" shell \
      'ping -c 1 -W 3 8.8.8.8 >/dev/null 2>&1 && ping -c 1 -W 3 google.com >/dev/null 2>&1' \
      >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "Android device ${serial} does not have working public internet/DNS; public-relay E2E cannot run reliably." >&2
  return 1
}
