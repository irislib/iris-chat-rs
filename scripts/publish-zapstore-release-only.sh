#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

bool_is_true() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

config_path="${ZAPSTORE_CONFIG:-$ROOT/zapstore.yaml}"
extra_flags="${ZSP_EXTRA_FLAGS:-}"
release_only="${IRIS_ZAPSTORE_RELEASE_ONLY:-true}"
release_pubkey="${IRIS_ZAPSTORE_RELEASE_PUBKEY:-}"
tmp_config=""

cleanup() {
  [[ -n "$tmp_config" ]] && rm -f "$tmp_config"
}
trap cleanup EXIT

if bool_is_true "$release_only"; then
  if [[ -z "$release_pubkey" ]]; then
    echo "IRIS_ZAPSTORE_RELEASE_PUBKEY is required for release-only Zapstore publish." >&2
    exit 2
  fi
  tmp_config="$(mktemp "$ROOT/.zapstore-release.XXXXXX.yaml")"
  sed "s#^pubkey: .*#pubkey: $release_pubkey#" "$config_path" > "$tmp_config"
  config_path="$tmp_config"
  case " $extra_flags " in
    *" --skip-app-event "*) ;;
    *) extra_flags="${extra_flags:+$extra_flags }--skip-app-event" ;;
  esac
  echo "Zapstore release-only publish: skipping app metadata event; signer $release_pubkey"
fi

ZAPSTORE_CONFIG="$config_path" \
  ZSP_EXTRA_FLAGS="$extra_flags" \
  "$ROOT/scripts/publish-zapstore-android.sh" "$@"
