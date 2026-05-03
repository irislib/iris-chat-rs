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
