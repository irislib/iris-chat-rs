#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/release_common.sh"

load_release_env "$ROOT"

ZAPSTORE_ENV_FILE="${ZAPSTORE_ENV_FILE:-$ROOT/.env.zapstore.local}"
if [[ -f "$ZAPSTORE_ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ZAPSTORE_ENV_FILE"
  set +a
fi

NDR_RELEASE_KEYSTORE_PATH="${NDR_RELEASE_KEYSTORE_PATH:-${ANDROID_KEYSTORE_PATH:-}}"
NDR_RELEASE_KEYSTORE_PASSWORD="${NDR_RELEASE_KEYSTORE_PASSWORD:-${ANDROID_KEYSTORE_PASSWORD:-}}"
NDR_RELEASE_KEY_ALIAS="${NDR_RELEASE_KEY_ALIAS:-${ANDROID_KEY_ALIAS:-}}"
NDR_RELEASE_KEY_PASSWORD="${NDR_RELEASE_KEY_PASSWORD:-${ANDROID_KEY_PASSWORD:-${ANDROID_KEYSTORE_PASSWORD:-}}}"
export NDR_RELEASE_KEYSTORE_PATH
export NDR_RELEASE_KEYSTORE_PASSWORD
export NDR_RELEASE_KEY_ALIAS
export NDR_RELEASE_KEY_PASSWORD

resolve_shared_build_metadata "$ROOT"

ZAPSTORE_CONFIG="${ZAPSTORE_CONFIG:-$ROOT/zapstore.yaml}"
ZAPSTORE_CHANNEL="${ZAPSTORE_CHANNEL:-main}"
ZAPSTORE_IDENTITY_RELAYS="${ZAPSTORE_IDENTITY_RELAYS:-wss://relay.zapstore.dev}"
SIGN_WITH="${SIGN_WITH:-browser}"
APK_PATH="$ROOT/dist/android/IrisChat-release-latest.apk"

TEMP_DIR=""
TEMP_P12_PATH=""
TEMP_IDENTITY_EVENT_PATH=""

usage() {
  cat <<'EOF'
usage: ./scripts/publish-zapstore-android.sh <print-config|doctor|build|check|link-identity|wizard|publish>
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

ensure_config() {
  if [[ ! -f "$ZAPSTORE_CONFIG" ]]; then
    echo "Missing Zapstore config: $ZAPSTORE_CONFIG" >&2
    exit 1
  fi
}

ensure_release_signing() {
  require_var NDR_RELEASE_KEYSTORE_PATH
  require_var NDR_RELEASE_KEYSTORE_PASSWORD
  require_var NDR_RELEASE_KEY_ALIAS
  require_var NDR_RELEASE_KEY_PASSWORD
  if [[ ! -f "$NDR_RELEASE_KEYSTORE_PATH" ]]; then
    echo "Release keystore not found: $NDR_RELEASE_KEYSTORE_PATH" >&2
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

export_pkcs12() {
  ensure_release_signing
  require_cmd keytool

  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-chat-zapstore-XXXXXX")"
  TEMP_P12_PATH="$TEMP_DIR/release-signing.p12"

  keytool -importkeystore \
    -noprompt \
    -srckeystore "$NDR_RELEASE_KEYSTORE_PATH" \
    -srcstoretype JKS \
    -srcstorepass "$NDR_RELEASE_KEYSTORE_PASSWORD" \
    -srcalias "$NDR_RELEASE_KEY_ALIAS" \
    -srckeypass "$NDR_RELEASE_KEY_PASSWORD" \
    -destkeystore "$TEMP_P12_PATH" \
    -deststoretype PKCS12 \
    -deststorepass "$NDR_RELEASE_KEYSTORE_PASSWORD" \
    -destkeypass "$NDR_RELEASE_KEYSTORE_PASSWORD" \
    -destalias "$NDR_RELEASE_KEY_ALIAS" >/dev/null
}

print_config() {
  cat <<EOF
zapstore.config=$ZAPSTORE_CONFIG
zapstore.channel=$ZAPSTORE_CHANNEL
zapstore.sign_with=$SIGN_WITH
zapstore.identity.relays=$ZAPSTORE_IDENTITY_RELAYS
release.keystore.path=${NDR_RELEASE_KEYSTORE_PATH:-}
release.apk.path=$APK_PATH
release.version.name=$NDR_APP_VERSION_NAME
release.version.code=$NDR_APP_VERSION_CODE
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
    -keystore "$NDR_RELEASE_KEYSTORE_PATH" \
    -storepass "$NDR_RELEASE_KEYSTORE_PASSWORD" \
    -alias "$NDR_RELEASE_KEY_ALIAS" >/dev/null

  cat <<EOF
zapstore.config=ok
zapstore.local.env=ok
zapstore.signing.method=$(signing_method_label)
android.release.env=ok
android.keystore=ok
android.key.alias=$NDR_RELEASE_KEY_ALIAS
android.app.id=to.iris.chat
EOF
}

check_publish_config() {
  ensure_config
  build_release_apk
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
  KEYSTORE_PASSWORD="$NDR_RELEASE_KEYSTORE_PASSWORD" \
    SIGN_WITH="$SIGN_WITH" \
    zsp identity --link-key "$TEMP_P12_PATH" --relays "$ZAPSTORE_IDENTITY_RELAYS" --offline > "$TEMP_IDENTITY_EVENT_PATH"
  nak event "$ZAPSTORE_IDENTITY_RELAYS" < "$TEMP_IDENTITY_EVENT_PATH"
}

run_publish() {
  local mode="$1"
  local cmd=(zsp publish "$ZAPSTORE_CONFIG" --channel "$ZAPSTORE_CHANNEL")
  local extra_flags=()

  ensure_config
  build_release_apk
  require_cmd zsp

  if [[ "$mode" == "wizard" ]]; then
    cmd=(zsp publish --wizard "$ZAPSTORE_CONFIG" --channel "$ZAPSTORE_CHANNEL")
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
    build_release_apk
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
