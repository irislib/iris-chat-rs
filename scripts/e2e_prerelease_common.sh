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
