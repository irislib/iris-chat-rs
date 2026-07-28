#!/usr/bin/env bash

release_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
}

load_release_env() {
  local root="$1"
  local env_file="${IRIS_RELEASE_ENV_FILE:-$root/release.env}"
  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

bool_is_true() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

epoch_to_iso8601() {
  local epoch="$1"
  if date -u -r 0 +"%Y-%m-%dT%H:%M:%SZ" >/dev/null 2>&1; then
    date -u -r "$epoch" +"%Y-%m-%dT%H:%M:%SZ"
  else
    date -u -d "@$epoch" +"%Y-%m-%dT%H:%M:%SZ"
  fi
}

git_short_sha() {
  local root="$1"
  git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || printf '%s\n' "unknown"
}

git_commit_timestamp_utc() {
  local root="$1"
  local epoch
  epoch="$(git -C "$root" log -1 --format=%ct HEAD 2>/dev/null || printf '%s' "")"
  if [[ -n "$epoch" ]]; then
    epoch_to_iso8601 "$epoch"
  else
    printf '%s\n' ""
  fi
}

semantic_version_code() {
  local version="$1"
  local core major minor patch build code

  core="${version%%[-+]*}"
  if [[ ! "$core" =~ ^([0-9]+)(\.([0-9]+))?(\.([0-9]+))?(\.([0-9]+))?$ ]]; then
    return 1
  fi

  major="${BASH_REMATCH[1]}"
  minor="${BASH_REMATCH[3]:-0}"
  patch="${BASH_REMATCH[5]:-0}"
  build="${BASH_REMATCH[7]:-0}"

  # Reserve two decimal digits for every component after major. For date-based
  # versions this is YYYYMMDDbb, where bb is an optional same-day build.
  # The previous packing overlapped month and day values (2026.7.14 and
  # 2026.8.4 both became 20268400) and fell behind older run-number overrides.
  if (( ${#major} > 4 || 10#$major > 2100 || 10#$minor > 99 || 10#$patch > 99 || 10#$build > 99 )); then
    return 1
  fi

  code="$((10#$major * 1000000 + 10#$minor * 10000 + 10#$patch * 100 + 10#$build))"
  if (( code < 1 || code > 2100000000 )); then
    return 1
  fi

  printf '%d\n' "$code"
}

# Apple's CFBundleShortVersionString accepts at most three integer components.
# The optional fourth ".build" segment we use to keep zapstore versions unique
# has to be stripped before handing the version to Xcode.
apple_marketing_version() {
  local version="$1"
  local core
  local a b c rest
  core="${version%%[-+]*}"
  IFS=. read -r a b c rest <<< "$core"
  if [[ -n "${rest:-}" ]]; then
    printf '%s.%s.%s\n' "${a:-0}" "${b:-0}" "${c:-0}"
    return
  fi
  printf '%s\n' "$core"
}

resolve_shared_build_metadata() {
  local root="$1"
  local derived_version_code

  IRIS_APP_VERSION_NAME="${IRIS_APP_VERSION_NAME:-0.1.0}"
  derived_version_code="$(semantic_version_code "$IRIS_APP_VERSION_NAME" || true)"
  if [[ -z "${IRIS_APP_VERSION_CODE:-}" ]]; then
    IRIS_APP_VERSION_CODE="${derived_version_code:-1}"
  elif [[ -n "${derived_version_code:-}" && "$IRIS_APP_VERSION_CODE" != "$derived_version_code" ]] && ! bool_is_true "${IRIS_APP_VERSION_CODE_MANUAL:-false}"; then
    echo "Using derived version code $derived_version_code for $IRIS_APP_VERSION_NAME (was $IRIS_APP_VERSION_CODE)." >&2
    IRIS_APP_VERSION_CODE="$derived_version_code"
  fi
  IRIS_BUILD_GIT_SHA="${IRIS_BUILD_GIT_SHA:-$(git_short_sha "$root")}"

  if [[ -z "${IRIS_BUILD_TIMESTAMP_UTC:-}" ]]; then
    if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
      IRIS_BUILD_TIMESTAMP_UTC="$(epoch_to_iso8601 "$SOURCE_DATE_EPOCH")"
    else
      IRIS_BUILD_TIMESTAMP_UTC="$(git_commit_timestamp_utc "$root")"
    fi
  fi

  if [[ -z "${IRIS_BUILD_TIMESTAMP_UTC:-}" ]]; then
    IRIS_BUILD_TIMESTAMP_UTC="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  fi
  IRIS_XCODE_MARKETING_VERSION="$(apple_marketing_version "$IRIS_APP_VERSION_NAME")"

  export IRIS_APP_VERSION_NAME
  export IRIS_APP_VERSION_CODE
  export IRIS_BUILD_GIT_SHA
  export IRIS_BUILD_TIMESTAMP_UTC
  export IRIS_XCODE_MARKETING_VERSION
}

release_slug() {
  local channel="$1"
  printf 'IrisChat-%s-%s+%s-%s' \
    "$channel" \
    "$IRIS_APP_VERSION_NAME" \
    "$IRIS_APP_VERSION_CODE" \
    "$IRIS_BUILD_GIT_SHA"
}

ensure_dir() {
  mkdir -p "$1"
}

copy_file_unless_same_file() {
  local source="$1"
  local destination="$2"

  if [[ -e "$destination" && "$source" -ef "$destination" ]]; then
    return 0
  fi
  cp "$source" "$destination"
}

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "$name must be set" >&2
    return 1
  fi
}

write_manifest() {
  local path="$1"
  shift

  : > "$path"
  while [[ $# -gt 1 ]]; do
    printf '%s=%s\n' "$1" "$2" >> "$path"
    shift 2
  done
}

windows_ssh_host_candidates() {
  local raw host
  if [[ -n "${IRIS_WINDOWS_SSH_HOST:-}" ]]; then
    printf '%s\n' "$IRIS_WINDOWS_SSH_HOST"
    return
  fi

  raw="${IRIS_WINDOWS_SSH_HOSTS:-windows-build win11-dev}"
  raw="${raw//,/ }"
  for host in $raw; do
    [[ -n "$host" ]] && printf '%s\n' "$host"
  done
}

windows_ssh_host_candidates_text() {
  local host text=""
  while IFS= read -r host; do
    [[ -n "$host" ]] || continue
    text="${text:+$text, }$host"
  done < <(windows_ssh_host_candidates)
  printf '%s\n' "$text"
}

select_windows_ssh_host() {
  local timeout="${1:-10}" host
  while IFS= read -r host; do
    [[ -n "$host" ]] || continue
    if ssh -n -o BatchMode=yes -o ConnectTimeout="$timeout" "$host" whoami >/dev/null 2>&1; then
      printf '%s\n' "$host"
      return 0
    fi
  done < <(windows_ssh_host_candidates)
  return 1
}
