#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FLOW="F01"
SETUP="ios"
RELAY="public"
FRESH=0
SKIP_BUILD=0
HEADLESS=0
EXTRA_ARGS=()

usage() {
  cat <<'EOF'
Usage: scripts/e2e_prerelease_matrix.sh [options]

Options:
  --list                 List documented pre-release flows.
  --flow ID              Flow id, for example F01. Default: F01.
  --setup SETUP          ios, android, mixed, or runtime. Default: ios.
  --relay public|local   Relay mode passed to mobile scripts. Default: public.
  --fresh                Reset app/keychain/package state where applicable.
  --skip-build           Reuse installed mobile artifacts.
  --headless             Launch Android emulators headlessly where applicable.
  --                     Pass remaining arguments through to the selected script.
  -h, --help             Show this help.

Only F01/F02 are fully scripted for mobile today. The remaining flows are tracked
in docs/e2e-prerelease-test-matrix.md and should be added here as they become
fully automated.
EOF
}

list_flows() {
  cat <<'EOF'
F01 fresh four-device baseline
F02 cross-platform linked user
F03 restart after link authorization
F04 restart after prepared direct send
F05 restart after group creation
F06 offline relay recovery
F07 delayed AppKeys/invite gaps
F08 group add/remove members
F09 group admin changes
F10 linked-device revocation
F11 delayed sender-key distribution
F12 app-level parity flow
F13 backfill/self-sync edge
F14 multi-restart soak
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      list_flows
      exit 0
      ;;
    --flow)
      FLOW="$2"
      shift 2
      ;;
    --setup)
      SETUP="$2"
      shift 2
      ;;
    --relay)
      RELAY="$2"
      shift 2
      ;;
    --fresh)
      FRESH=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --headless)
      HEADLESS=1
      shift
      ;;
    --)
      shift
      EXTRA_ARGS+=("$@")
      break
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "${SETUP}" in
  ios|android|mixed|runtime) ;;
  *)
    echo "Unknown setup: ${SETUP}" >&2
    exit 1
    ;;
esac

if [[ "${SETUP}" == "runtime" ]]; then
  case "${FLOW}" in
    F01|F03|F04|F05|F07|F08|F09|F10|F11|F12|F13)
      (
        cd "${ROOT_DIR}/../nostr-double-ratchet/rust" &&
          cargo test -p nostr-double-ratchet prerelease
      )
      (
        cd "${ROOT_DIR}/core" &&
          cargo test prerelease
      )
      exit 0
      ;;
    *)
      echo "Runtime flow ${FLOW} is documented but not wired to a named runtime test yet." >&2
      exit 2
      ;;
  esac
fi

SCRIPT_ARGS=(--relay "${RELAY}")
[[ "${FRESH}" -eq 1 ]] && SCRIPT_ARGS+=(--fresh)
[[ "${SKIP_BUILD}" -eq 1 ]] && SCRIPT_ARGS+=(--skip-build)
[[ "${HEADLESS}" -eq 1 ]] && SCRIPT_ARGS+=(--headless)
if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
  SCRIPT_ARGS+=("${EXTRA_ARGS[@]}")
fi

case "${FLOW}:${SETUP}" in
  F01:ios)
    exec "${ROOT_DIR}/scripts/e2e_ios_public_prerelease.sh" "${SCRIPT_ARGS[@]}"
    ;;
  F01:android)
    exec "${ROOT_DIR}/scripts/e2e_android_public_prerelease.sh" "${SCRIPT_ARGS[@]}"
    ;;
  F01:mixed|F02:mixed)
    exec "${ROOT_DIR}/scripts/e2e_mixed_public_prerelease.sh" "${SCRIPT_ARGS[@]}"
    ;;
  F02:ios|F02:android)
    echo "Flow F02 requires --setup mixed because the linked user spans both platforms." >&2
    exit 2
    ;;
  *)
    echo "Flow ${FLOW} for setup ${SETUP} is documented but not fully scripted yet." >&2
    echo "See docs/e2e-prerelease-test-matrix.md." >&2
    exit 2
    ;;
esac
