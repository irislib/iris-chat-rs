#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="${IRIS_RELEASE_GATE_ROOT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
CONFIG="${IRIS_RELEASE_GATE_CONFIG:-}"
TTL_SECONDS="${IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS:-43200}"
NOW="${IRIS_RELEASE_GATE_RECEIPT_NOW:-$(date +%s)}"

usage() {
  echo "usage: release_gate_receipt.sh <check|write|path>" >&2
}

sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  else
    openssl dgst -sha256 | awk '{print $NF}'
  fi
}

require_context() {
  [[ -n "$CONFIG" ]] || {
    echo "IRIS_RELEASE_GATE_CONFIG is required." >&2
    return 2
  }
  [[ "$TTL_SECONDS" =~ ^[1-9][0-9]*$ ]] || {
    echo "IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS must be a positive integer." >&2
    return 2
  }
  [[ "$NOW" =~ ^[0-9]+$ ]] || {
    echo "IRIS_RELEASE_GATE_RECEIPT_NOW must be an integer." >&2
    return 2
  }
  COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null)" || return 2
  CONFIG_SHA="$(printf '%s' "$CONFIG" | sha256)"
  local common_dir
  common_dir="$(git -C "$ROOT_DIR" rev-parse --git-common-dir)"
  if [[ "$common_dir" != /* ]]; then
    common_dir="$ROOT_DIR/$common_dir"
  fi
  RECEIPT_DIR="$common_dir/iris-release-gates/$COMMIT"
  RECEIPT_PATH="$RECEIPT_DIR/$CONFIG_SHA.receipt"
}

worktree_is_clean() {
  [[ -z "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=normal)" ]]
}

payload() {
  local created_at="$1"
  printf 'version=1\ncommit=%s\nconfig_sha256=%s\ncreated_at=%s\n' \
    "$COMMIT" "$CONFIG_SHA" "$created_at"
}

check_receipt() {
  require_context
  worktree_is_clean || return 1
  [[ -f "$RECEIPT_PATH" ]] || return 1

  local version commit config_sha created_at checksum expected_checksum age
  version="$(sed -n '1s/^version=//p' "$RECEIPT_PATH")"
  commit="$(sed -n '2s/^commit=//p' "$RECEIPT_PATH")"
  config_sha="$(sed -n '3s/^config_sha256=//p' "$RECEIPT_PATH")"
  created_at="$(sed -n '4s/^created_at=//p' "$RECEIPT_PATH")"
  checksum="$(sed -n '5s/^checksum=//p' "$RECEIPT_PATH")"

  [[ "$(wc -l < "$RECEIPT_PATH" | tr -d ' ')" == "5" ]] || return 1
  [[ "$version" == "1" && "$commit" == "$COMMIT" ]] || return 1
  [[ "$config_sha" == "$CONFIG_SHA" && "$created_at" =~ ^[0-9]+$ ]] || return 1
  expected_checksum="$(payload "$created_at" | sha256)"
  [[ "$checksum" == "$expected_checksum" ]] || return 1
  (( NOW >= created_at )) || return 1
  age=$((NOW - created_at))
  (( age <= TTL_SECONDS ))
}

write_receipt() {
  require_context
  worktree_is_clean || return 1

  mkdir -p "$RECEIPT_DIR"
  local checksum tmp
  checksum="$(payload "$NOW" | sha256)"
  tmp="$(mktemp "$RECEIPT_PATH.tmp.XXXXXX")"
  {
    payload "$NOW"
    printf 'checksum=%s\n' "$checksum"
  } > "$tmp"
  mv "$tmp" "$RECEIPT_PATH"
}

case "${1:-}" in
  check) check_receipt ;;
  write) write_receipt ;;
  path)
    require_context
    printf '%s\n' "$RECEIPT_PATH"
    ;;
  *)
    usage
    exit 2
    ;;
esac
