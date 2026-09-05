#!/usr/bin/env bash

# Variables in this sourced file are consumed by scripts/distribute.
# shellcheck disable=SC2034
IRIS_GITHUB_REPOSITORY="irislib/iris-chat-rs"
IRIS_HASHTREE_PUBLISHER_NPUB="npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap"
IRIS_ZAPSTORE_PUBLISHER_NPUB="npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm"
IRIS_HASHTREE_RELEASE_TREE="releases/iris-chat-rs"

IRIS_HASHTREE_CONFIG_DIR="${IRIS_HASHTREE_CONFIG_DIR:-$HOME/.config/iris-chat/htree-release-config}"
IRIS_HASHTREE_DATA_DIR="${IRIS_HASHTREE_DATA_DIR:-$HOME/.config/iris-chat/htree-release-data}"
IRIS_HASHTREE_NSEC_PATH="${IRIS_HASHTREE_NSEC_PATH:-$HOME/.config/iris-chat/htree-nsec}"
IRIS_ZAPSTORE_NSEC_PATH="${IRIS_ZAPSTORE_NSEC_PATH:-$HOME/.config/iris-chat/zapstore-nsec}"

require_command() {
  local command_name="$1"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    return 1
  fi
}

read_nsec() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "Missing local signer file: $path" >&2
    return 1
  fi
  awk 'NF { print $1; exit }' "$path"
}

npub_for_nsec_path() {
  local path="$1"
  local nsec public_hex
  nsec="$(read_nsec "$path")" || return 1
  public_hex="$(nak key public "$nsec" 2>/dev/null)" || {
    echo "Could not derive a public key from $path" >&2
    return 1
  }
  nak encode npub "$public_hex"
}

require_signer() {
  local label="$1"
  local path="$2"
  local expected="$3"
  local actual
  require_command nak
  actual="$(npub_for_nsec_path "$path")" || return 1
  if [[ "$actual" != "$expected" ]]; then
    echo "$label signer mismatch." >&2
    echo "Expected: $expected" >&2
    echo "Actual:   $actual" >&2
    echo "Key file: $path" >&2
    return 1
  fi
}

require_hashtree_identity() {
  local active
  require_command htree
  require_signer "Hashtree" "$IRIS_HASHTREE_NSEC_PATH" "$IRIS_HASHTREE_PUBLISHER_NPUB"
  active="$(
    HTREE_CONFIG_DIR="$IRIS_HASHTREE_CONFIG_DIR" \
    HTREE_DATA_DIR="$IRIS_HASHTREE_DATA_DIR" htree user 2>/dev/null |
      grep -oE 'npub1[023456789acdefghjklmnpqrstuvwxyz]+' |
      head -n 1
  )"
  if [[ "$active" != "$IRIS_HASHTREE_PUBLISHER_NPUB" ]]; then
    echo "Active Hashtree identity mismatch in $IRIS_HASHTREE_CONFIG_DIR." >&2
    echo "Expected: $IRIS_HASHTREE_PUBLISHER_NPUB" >&2
    echo "Actual:   ${active:-<none>}" >&2
    return 1
  fi
}

require_zapstore_identity() {
  require_command zsp
  require_signer "Zapstore" "$IRIS_ZAPSTORE_NSEC_PATH" "$IRIS_ZAPSTORE_PUBLISHER_NPUB"
}

hashtree_gateway_base_url() {
  local tag="$1"
  printf 'https://upload.iris.to/%s/releases%%2Firis-chat-rs/%s\n' \
    "$IRIS_HASHTREE_PUBLISHER_NPUB" \
    "$tag"
}
