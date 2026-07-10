#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_executable() {
  local path="$1"
  [[ -x "$ROOT/$path" ]] || { echo "missing executable: $path" >&2; exit 1; }
}

require_contains() {
  local path="$1"
  local text="$2"
  grep -Fq -- "$text" "$ROOT/$path" || {
    echo "missing '$text' in $path" >&2
    exit 1
  }
}

require_executable scripts/verify.sh
require_executable scripts/verify_full_native.sh
require_executable scripts/native_lab.py
require_executable scripts/native_state_reset.sh
require_contains justfile "verify-fast:"
require_contains justfile "verify-full:"
require_contains justfile "verify-health:"
require_contains scripts/verify.sh 'cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings'
require_contains scripts/verify_full_native.sh "--on-device"
require_contains scripts/test-release-gate "--skip-fast"

bash -n \
  "$ROOT/scripts/verify.sh" \
  "$ROOT/scripts/verify_full_native.sh" \
  "$ROOT/scripts/native_state_reset.sh"

echo "verification tier contract passed"
