#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/release_common.sh"

load_release_env "$ROOT"

DEFAULT_ZAPSTORE_ENV_FILE="$ROOT/.env.zapstore.local"
if [[ ! -f "$DEFAULT_ZAPSTORE_ENV_FILE" && -f "$ROOT/../nostr-vpn/.env.zapstore.local" ]]; then
  DEFAULT_ZAPSTORE_ENV_FILE="$ROOT/../nostr-vpn/.env.zapstore.local"
fi
ZAPSTORE_ENV_FILE="${ZAPSTORE_ENV_FILE:-$DEFAULT_ZAPSTORE_ENV_FILE}"
if [[ -f "$ZAPSTORE_ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ZAPSTORE_ENV_FILE"
  set +a
fi

configure_release_htree_identity
if [[ -n "${IRIS_ZAPSTORE_SIGN_WITH:-}" ]]; then
  SIGN_WITH="$IRIS_ZAPSTORE_SIGN_WITH"
elif [[ -f "$IRIS_RELEASE_NOSTR_KEY_PATH" ]]; then
  unset SIGN_WITH
  NOSTR_KEY_PATH="$IRIS_RELEASE_NOSTR_KEY_PATH"
fi
export NOSTR_KEY_PATH

IRIS_RELEASE_KEYSTORE_PATH="${IRIS_RELEASE_KEYSTORE_PATH:-${ANDROID_KEYSTORE_PATH:-}}"
IRIS_RELEASE_KEYSTORE_PASSWORD="${IRIS_RELEASE_KEYSTORE_PASSWORD:-${ANDROID_KEYSTORE_PASSWORD:-}}"
IRIS_RELEASE_KEY_ALIAS="${IRIS_RELEASE_KEY_ALIAS:-${ANDROID_KEY_ALIAS:-}}"
IRIS_RELEASE_KEY_PASSWORD="${IRIS_RELEASE_KEY_PASSWORD:-${ANDROID_KEY_PASSWORD:-${ANDROID_KEYSTORE_PASSWORD:-}}}"
export IRIS_RELEASE_KEYSTORE_PATH
export IRIS_RELEASE_KEYSTORE_PASSWORD
export IRIS_RELEASE_KEY_ALIAS
export IRIS_RELEASE_KEY_PASSWORD

resolve_shared_build_metadata "$ROOT"

ZAPSTORE_CONFIG="${ZAPSTORE_CONFIG:-$ROOT/zapstore.yaml}"
ZAPSTORE_CHANNEL="${ZAPSTORE_CHANNEL:-main}"
ZAPSTORE_IDENTITY_RELAYS="${ZAPSTORE_IDENTITY_RELAYS:-wss://relay.zapstore.dev}"
APK_PATH="$ROOT/dist/android/IrisChat-release-latest.apk"
ZAPSTORE_APK_PATH="${ZAPSTORE_APK_PATH:-${IRIS_ZAPSTORE_APK_PATH:-}}"

TEMP_DIR=""
TEMP_P12_PATH=""
TEMP_IDENTITY_EVENT_PATH=""

usage() {
  cat <<'EOF'
usage: ./scripts/publish-zapstore-android.sh <print-config|doctor|build|check|link-identity|wizard|publish>

Environment:
  ZAPSTORE_APK_PATH         Use this already-signed APK instead of building locally.
                            The APK is copied to dist/android/IrisChat-release-latest.apk
                            because zapstore.yaml references that stable path.
  IRIS_RELEASE_NOSTR_KEY_PATH
                            Dedicated release signer file (default:
                            ~/.keys/irischat-release.nsec).
EOF
}

cleanup() {
  if [[ -n "${TEMP_P12_PATH}" && -f "${TEMP_P12_PATH}" ]]; then
    rm -f "${TEMP_P12_PATH}"
  fi
  if [[ -n "${TEMP_IDENTITY_EVENT_PATH}" && -f "${TEMP_IDENTITY_EVENT_PATH}" ]]; then
    rm -f "${TEMP_IDENTITY_EVENT_PATH}"
  fi
  if [[ -n "${TEMP_DIR}" && -d "${TEMP_DIR}" ]]; then
    rmdir "${TEMP_DIR}" 2>/dev/null || true
  fi
}

trap cleanup EXIT

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

resolve_sign_with() {
  if [[ -n "${SIGN_WITH:-}" ]]; then
    printf '%s\n' "$SIGN_WITH"
    return 0
  fi
  if [[ -n "${NOSTR_KEY_PATH:-}" && -f "$NOSTR_KEY_PATH" ]]; then
    tr -d '\r\n' < "$NOSTR_KEY_PATH"
    return 0
  fi
  printf '%s\n' "browser"
}

SIGN_WITH="$(resolve_sign_with)"
export SIGN_WITH

ensure_config() {
  if [[ ! -f "$ZAPSTORE_CONFIG" ]]; then
    echo "Missing Zapstore config: $ZAPSTORE_CONFIG" >&2
    exit 1
  fi
}

ensure_release_signing() {
  require_var IRIS_RELEASE_KEYSTORE_PATH
  require_var IRIS_RELEASE_KEYSTORE_PASSWORD
  require_var IRIS_RELEASE_KEY_ALIAS
  require_var IRIS_RELEASE_KEY_PASSWORD
  if [[ ! -f "$IRIS_RELEASE_KEYSTORE_PATH" ]]; then
    echo "Release keystore not found: $IRIS_RELEASE_KEYSTORE_PATH" >&2
    exit 1
  fi
}

build_release_apk() {
  ensure_release_signing
  "$ROOT/scripts/android-release" release-apk >/dev/null
  if [[ ! -f "$APK_PATH" ]]; then
    echo "Expected release APK at $APK_PATH" >&2
    exit 1
  fi
}

stage_existing_apk() {
  local source_path="$1"

  if [[ -z "$source_path" ]]; then
    echo "ZAPSTORE_APK_PATH must not be empty" >&2
    exit 1
  fi
  if [[ ! -f "$source_path" ]]; then
    echo "Existing Zapstore APK not found: $source_path" >&2
    exit 1
  fi

  ensure_dir "$(dirname "$APK_PATH")"
  if [[ "$(cd "$(dirname "$source_path")" && pwd)/$(basename "$source_path")" != "$APK_PATH" ]]; then
    cp "$source_path" "$APK_PATH"
  fi
  printf '%s\n' "$APK_PATH"
}

prepare_release_apk() {
  if [[ -n "${ZAPSTORE_APK_PATH:-}" ]]; then
    stage_existing_apk "$ZAPSTORE_APK_PATH" >/dev/null
  else
    build_release_apk
  fi
  if [[ ! -f "$APK_PATH" ]]; then
    echo "Expected release APK at $APK_PATH" >&2
    exit 1
  fi
}

export_pkcs12() {
  ensure_release_signing
  require_cmd keytool

  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-chat-zapstore-XXXXXX")"
  TEMP_P12_PATH="$TEMP_DIR/release-signing.p12"

  keytool -importkeystore \
    -noprompt \
    -srckeystore "$IRIS_RELEASE_KEYSTORE_PATH" \
    -srcstoretype JKS \
    -srcstorepass "$IRIS_RELEASE_KEYSTORE_PASSWORD" \
    -srcalias "$IRIS_RELEASE_KEY_ALIAS" \
    -srckeypass "$IRIS_RELEASE_KEY_PASSWORD" \
    -destkeystore "$TEMP_P12_PATH" \
    -deststoretype PKCS12 \
    -deststorepass "$IRIS_RELEASE_KEYSTORE_PASSWORD" \
    -destkeypass "$IRIS_RELEASE_KEYSTORE_PASSWORD" \
    -destalias "$IRIS_RELEASE_KEY_ALIAS" >/dev/null
}

print_config() {
  cat <<EOF
zapstore.config=$ZAPSTORE_CONFIG
zapstore.channel=$ZAPSTORE_CHANNEL
zapstore.signing.method=$(signing_method_label)
zapstore.identity.relays=$ZAPSTORE_IDENTITY_RELAYS
zapstore.env.file=$ZAPSTORE_ENV_FILE
release.keystore.path=${IRIS_RELEASE_KEYSTORE_PATH:-}
release.apk.source=${ZAPSTORE_APK_PATH:-build}
release.apk.path=$APK_PATH
release.version.name=$IRIS_APP_VERSION_NAME
release.version.code=$IRIS_APP_VERSION_CODE
EOF
}

signing_method_label() {
  case "$SIGN_WITH" in
    browser)
      printf '%s\n' "browser"
      ;;
    nsec1*)
      printf '%s\n' "nsec"
      ;;
    bunker://*)
      printf '%s\n' "bunker"
      ;;
    *)
      printf '%s\n' "custom"
      ;;
  esac
}

doctor() {
  ensure_config
  ensure_release_signing
  require_cmd keytool
  require_cmd zsp

  if [[ ! -f "$ZAPSTORE_ENV_FILE" ]]; then
    echo "Missing local Zapstore env file: $ZAPSTORE_ENV_FILE" >&2
    exit 1
  fi

  keytool -list \
    -keystore "$IRIS_RELEASE_KEYSTORE_PATH" \
    -storepass "$IRIS_RELEASE_KEYSTORE_PASSWORD" \
    -alias "$IRIS_RELEASE_KEY_ALIAS" >/dev/null

  cat <<EOF
zapstore.config=ok
zapstore.local.env=ok
zapstore.signing.method=$(signing_method_label)
android.release.env=ok
android.keystore=ok
android.key.alias=$IRIS_RELEASE_KEY_ALIAS
android.app.id=to.iris.chat
EOF
}

check_publish_config() {
  ensure_config
  prepare_release_apk
  require_cmd zsp
  zsp publish --check "$ZAPSTORE_CONFIG"
}

link_identity() {
  ensure_config
  build_release_apk
  require_cmd zsp
  require_cmd nak
  export_pkcs12
  TEMP_IDENTITY_EVENT_PATH="$TEMP_DIR/identity-event.json"
  KEYSTORE_PASSWORD="$IRIS_RELEASE_KEYSTORE_PASSWORD" \
    SIGN_WITH="$SIGN_WITH" \
    zsp identity --link-key "$TEMP_P12_PATH" --relays "$ZAPSTORE_IDENTITY_RELAYS" --offline > "$TEMP_IDENTITY_EVENT_PATH"
  nak event "$ZAPSTORE_IDENTITY_RELAYS" < "$TEMP_IDENTITY_EVENT_PATH"
}

run_publish() {
  local mode="$1"
  local cmd=(zsp publish "$ZAPSTORE_CONFIG" --channel "$ZAPSTORE_CHANNEL")
  local extra_flags=()

  ensure_config
  prepare_release_apk
  require_cmd zsp

  if [[ "$mode" == "wizard" ]]; then
    cmd=(zsp publish --wizard "$ZAPSTORE_CONFIG" --channel "$ZAPSTORE_CHANNEL")
  else
    cmd+=(--quiet --skip-preview --overwrite-release)
  fi

  if [[ -n "${ZSP_EXTRA_FLAGS:-}" ]]; then
    # shellcheck disable=SC2206
    extra_flags=(${ZSP_EXTRA_FLAGS})
    cmd+=("${extra_flags[@]}")
  fi

  SIGN_WITH="$SIGN_WITH" "${cmd[@]}"
}

case "${1:-}" in
  print-config)
    print_config
    ;;
  doctor)
    doctor
    ;;
  build)
    prepare_release_apk
    printf '%s\n' "$APK_PATH"
    ;;
  check)
    check_publish_config
    ;;
  link-identity)
    link_identity
    ;;
  wizard)
    run_publish wizard
    ;;
  publish)
    run_publish publish
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
