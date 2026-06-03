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

F01/F02 are scripted for the public mobile baseline. F08 iOS/Android,
F09 iOS/Android, F10 mixed, F11 iOS, F12, F13 mixed, F14 mixed, F15 iOS/Android/mixed, F16 mixed,
and F17 support public and local mobile coverage. F03, F04, F05, F06, F07, F08, F09,
F10, F11, F14, F15, and F16 have local-relay mobile harness coverage. The remaining flows are
tracked in docs/e2e-prerelease-test-matrix.md and should be added here as they
become fully automated.
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
F15 existing profile restore with secret key
F16 cold group invite
F17 mixed multi-device mesh
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
          cargo test --workspace
      )
      (
        cd "${ROOT_DIR}/core" &&
          cargo test
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
  F03:mixed)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F03 requires --relay local for deterministic link bootstrap restart recovery." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F03 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_link_restart_recovery.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_link_restart_recovery.py"
    ;;
  F04:mixed|F05:mixed)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow ${FLOW} requires --relay local so publish timing is deterministic." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow ${FLOW} creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    if [[ "${FLOW}" == "F04" ]]; then
      MIXED_ARGS=(--case direct)
    else
      MIXED_ARGS=(--case group)
    fi
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_publish_restart_recovery.py" "${MIXED_ARGS[@]}"
    ;;
  F06:ios)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F06 requires --relay local so the message server can be stopped and restarted." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F06 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    PROTOCOL_ARGS=(--case direct_group_offline_restart_recovery)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      PROTOCOL_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    exec python3 "${ROOT_DIR}/scripts/protocol_fault_validation.py" "${PROTOCOL_ARGS[@]}"
    ;;
  F06:android)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F06 requires --relay local so the message server can be stopped and restarted." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F06 creates a per-run local message server URL and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/android_offline_restart_recovery.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/android_offline_restart_recovery.py"
    ;;
  F06:mixed)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F06 requires --relay local so offline and restart recovery stays deterministic." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F06 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_offline_restart_recovery.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_offline_restart_recovery.py"
    ;;
  F07:ios)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F07 requires --relay local so metadata fault injection is deterministic." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F07 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    PROTOCOL_ARGS=(
      --case group_metadata_drop_then_multiple_messages
      --case relay_offline_outbox_then_repair
    )
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      PROTOCOL_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    exec python3 "${ROOT_DIR}/scripts/protocol_fault_validation.py" "${PROTOCOL_ARGS[@]}"
    ;;
  F08:mixed)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F08 requires --relay local for deterministic group membership propagation." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F08 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_group_membership_matrix.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_group_membership_matrix.py"
    ;;
  F08:ios)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F08 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    IOS_ARGS=(--platform ios --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && IOS_ARGS+=(--skip-build)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      IOS_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#IOS_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/group_membership_same_platform_flow.py" "${IOS_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/group_membership_same_platform_flow.py" --platform ios
    ;;
  F08:android)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F08 creates per-run local message server URLs and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=(--platform android --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && ANDROID_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/group_membership_same_platform_flow.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/group_membership_same_platform_flow.py" --platform android
    ;;
  F09:mixed)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F09 requires --relay local for deterministic group admin propagation." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F09 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_group_admin_matrix.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_group_admin_matrix.py"
    ;;
  F09:ios)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F09 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    IOS_ARGS=(--platform ios --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && IOS_ARGS+=(--skip-build)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      IOS_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#IOS_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/group_admin_same_platform_flow.py" "${IOS_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/group_admin_same_platform_flow.py" --platform ios
    ;;
  F09:android)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F09 creates per-run local message server URLs and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=(--platform android --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && ANDROID_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/group_admin_same_platform_flow.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/group_admin_same_platform_flow.py" --platform android
    ;;
  F10:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F10 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_linked_device_revocation.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_linked_device_revocation.py"
    ;;
  F11:ios)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F11 requires --relay local so sender-key fault injection is deterministic." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F11 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    PROTOCOL_ARGS=(--case sender_key_distribution_repair)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      PROTOCOL_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    exec python3 "${ROOT_DIR}/scripts/protocol_fault_validation.py" "${PROTOCOL_ARGS[@]}"
    ;;
  F12:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F12 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_app_parity_flow.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_app_parity_flow.py"
    ;;
  F12:ios)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F12 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    IOS_ARGS=(--platform ios --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && IOS_ARGS+=(--skip-build)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      IOS_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#IOS_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/app_parity_same_platform_flow.py" "${IOS_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/app_parity_same_platform_flow.py" --platform ios
    ;;
  F12:android)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F12 creates per-run local message server URLs and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=(--platform android --relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && ANDROID_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/app_parity_same_platform_flow.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/app_parity_same_platform_flow.py" --platform android
    ;;
  F13:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F13 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py"
    ;;
  F14:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F14 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_multi_restart_soak.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_multi_restart_soak.py"
    ;;
  F15:android)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F15 creates a per-run local message server URL and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && ANDROID_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/android_restore_existing_profile.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/android_restore_existing_profile.py"
    ;;
  F15:ios)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F15 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    IOS_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && IOS_ARGS+=(--skip-build)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      IOS_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#IOS_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/ios_restore_existing_profile.py" "${IOS_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/ios_restore_existing_profile.py"
    ;;
  F15:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F15 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_restore_existing_profile.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_restore_existing_profile.py"
    ;;
  F16:ios)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F16 requires --relay local for deterministic cold group invite delivery." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F16 creates a per-run local message server URL and must rebuild the iOS harness." >&2
      exit 2
    fi
    IOS_ARGS=()
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      IOS_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#IOS_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/ios_cold_group_invite.py" "${IOS_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/ios_cold_group_invite.py"
    ;;
  F16:android)
    if [[ "${RELAY}" != "local" ]]; then
      echo "Flow F16 requires --relay local for deterministic cold group invite delivery." >&2
      exit 2
    fi
    if [[ "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F16 creates a per-run local message server URL and must rebuild the Android app." >&2
      exit 2
    fi
    ANDROID_ARGS=()
    [[ "${HEADLESS}" -eq 1 ]] && ANDROID_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      ANDROID_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#ANDROID_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/android_cold_group_invite.py" "${ANDROID_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/android_cold_group_invite.py"
    ;;
  F16:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F16 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_cold_group_invite.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_cold_group_invite.py"
    ;;
  F17:mixed)
    if [[ "${RELAY}" == "local" && "${SKIP_BUILD}" -eq 1 ]]; then
      echo "Flow F17 creates per-run local message server URLs and must rebuild the mobile apps." >&2
      exit 2
    fi
    MIXED_ARGS=(--relay-mode "${RELAY}")
    [[ "${SKIP_BUILD}" -eq 1 ]] && MIXED_ARGS+=(--skip-build)
    [[ "${HEADLESS}" -eq 1 ]] && MIXED_ARGS+=(--headless)
    if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
      MIXED_ARGS+=("${EXTRA_ARGS[@]}")
    fi
    if [[ ${#MIXED_ARGS[@]} -gt 0 ]]; then
      exec python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py" "${MIXED_ARGS[@]}"
    fi
    exec python3 "${ROOT_DIR}/scripts/mixed_multi_device_mesh.py"
    ;;
  *)
    echo "Flow ${FLOW} for setup ${SETUP} is documented but not fully scripted yet." >&2
    echo "See docs/e2e-prerelease-test-matrix.md." >&2
    exit 2
    ;;
esac
