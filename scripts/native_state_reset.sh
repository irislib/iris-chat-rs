#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'EOF'
usage:
  IRIS_NATIVE_LAB_ALLOW_RESET=1 scripts/native_state_reset.sh ios-simulator \
    --udid <udid> [--bundle-id <id> ...] [--erase]
  IRIS_NATIVE_LAB_ALLOW_RESET=1 scripts/native_state_reset.sh android \
    --serial <serial> --bundle-id <id> [--test-bundle-id <id>]

Use only while scripts/native_lab.py holds the matching resource reservation.
Simulator --erase is intended for dedicated lab simulators.
EOF
}

if [[ "${IRIS_NATIVE_LAB_ALLOW_RESET:-0}" != "1" ]]; then
  echo "native state reset requires IRIS_NATIVE_LAB_ALLOW_RESET=1" >&2
  exit 75
fi

reset_ios() {
  local udid=""
  local erase=0
  local -a bundle_ids=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --udid) udid="$2"; shift 2 ;;
      --bundle-id) bundle_ids+=("$2"); shift 2 ;;
      --erase) erase=1; shift ;;
      *) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$udid" ]] || { usage >&2; exit 2; }
  xcrun simctl list devices available --json | python3 -c \
    'import json,sys; u=sys.argv[1]; d=json.load(sys.stdin).get("devices",{}); raise SystemExit(0 if any(x.get("udid")==u and x.get("isAvailable") for xs in d.values() for x in xs) else 75)' \
    "$udid"
  xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  if [[ "$erase" == "1" ]]; then
    xcrun simctl erase "$udid" || exit 75
  else
    local bundle_id
    for bundle_id in "${bundle_ids[@]}"; do
      xcrun simctl uninstall "$udid" "$bundle_id" >/dev/null 2>&1 || true
    done
  fi
  xcrun simctl boot "$udid" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$udid" -b >/dev/null || exit 75
  printf 'reset ios-simulator %s\n' "$udid"
}

reset_android() {
  local serial=""
  local bundle_id=""
  local test_bundle_id=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --serial) serial="$2"; shift 2 ;;
      --bundle-id) bundle_id="$2"; shift 2 ;;
      --test-bundle-id) test_bundle_id="$2"; shift 2 ;;
      *) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$serial" && -n "$bundle_id" ]] || { usage >&2; exit 2; }
  adb -s "$serial" get-state 2>/dev/null | grep -qx device || exit 75
  adb -s "$serial" shell am force-stop "$bundle_id" >/dev/null 2>&1 || true
  adb -s "$serial" shell pm clear "$bundle_id" >/dev/null || exit 75
  if [[ -n "$test_bundle_id" ]]; then
    adb -s "$serial" shell am force-stop "$test_bundle_id" >/dev/null 2>&1 || true
    adb -s "$serial" shell pm clear "$test_bundle_id" >/dev/null 2>&1 || true
  fi
  printf 'reset android %s %s\n' "$serial" "$bundle_id"
}

case "${1:-}" in
  ios-simulator) shift; reset_ios "$@" ;;
  android) shift; reset_android "$@" ;;
  *) usage >&2; exit 2 ;;
esac
