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
  local core major minor patch build

  core="${version%%[-+]*}"
  if [[ ! "$core" =~ ^([0-9]+)(\.([0-9]+))?(\.([0-9]+))?(\.([0-9]+))?$ ]]; then
    return 1
  fi

  major="${BASH_REMATCH[1]}"
  minor="${BASH_REMATCH[3]:-0}"
  patch="${BASH_REMATCH[5]:-0}"
  build="${BASH_REMATCH[7]:-0}"

  printf '%d\n' "$((10#$major * 10000 + 10#$minor * 1000 + 10#$patch * 100 + 10#$build))"
}

# Apple's CFBundleShortVersionString accepts at most three integer components.
# The optional fourth ".build" segment we use to keep zapstore versions unique
# has to be stripped before handing the version to Xcode.
apple_marketing_version() {
  local version="$1"
  local core
  core="${version%%[-+]*}"
  IFS=. read -r a b c _rest <<< "$core"
  printf '%s.%s.%s\n' "${a:-0}" "${b:-0}" "${c:-0}"
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
