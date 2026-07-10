#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The retired machine-wide target made unrelated worktrees serialize on Cargo's
# artifact lock. Keep an explicitly configured target, but do not inherit that
# legacy shared path from a long-lived shell or app process.
if [[ "${CARGO_TARGET_DIR:-}" == "$HOME/.cache/cargo-target" ]]; then
  unset CARGO_TARGET_DIR
fi
if command -v sccache >/dev/null 2>&1; then
  export SCCACHE_BASEDIRS="${SCCACHE_BASEDIRS:-$ROOT}"
fi

usage() {
  cat <<'EOF'
usage: scripts/verify.sh fast|full|health

fast   Per-change Rust/core/contract checks without native devices or GUI.
full   Fast checks plus reserved five-platform and physical-device matrices.
health Preflight full-matrix resources without running tests.

Full verification requires an explicit IRIS_WINDOWS_SSH_HOST plus a usable
iOS simulator, paired iOS device, authorized Android device, Docker, and the
local macOS toolchain. Set IRIS_NATIVE_LAB_RESET=1 only when the selected
simulator and Android target are dedicated lab devices.
EOF
}

run_fast() {
  python3 scripts/test_native_lab.py
  cargo fmt --manifest-path core/Cargo.toml --check
  cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings
  scripts/test_fast.sh --core-only
}

build_health_args() {
  HEALTH_ARGS=(
    --health local:macos
    --health command:xcrun
    --health command:xcodebuild
    --health command:xcodegen
    --health command:adb
    --health docker:daemon
    --health command:ssh
    --health "env:IRIS_WINDOWS_SSH_HOST"
    --health "ios-simulator:${IRIS_CHAT_LAB_IOS_SIMULATOR:-auto}"
    --health "ios-device:${IRIS_CHAT_LAB_IOS_DEVICE:-auto}"
    --health "android:${IRIS_CHAT_LAB_ANDROID_SERIAL:-auto}"
  )
  if [[ -n "${IRIS_WINDOWS_SSH_HOST:-}" ]]; then
    HEALTH_ARGS+=(--health "ssh:${IRIS_WINDOWS_SSH_HOST}")
  fi
  ALLOCATION_ARGS=(
    --allocation-env ios-simulator=IRIS_CHAT_LAB_ALLOCATED_IOS_SIMULATOR
    --allocation-env ios-device=IRIS_CHAT_LAB_ALLOCATED_IOS_DEVICE
    --allocation-env android=IRIS_CHAT_LAB_ALLOCATED_ANDROID
  )
}

run_managed_full() {
  build_health_args
  result="${IRIS_CHAT_VERIFY_RESULT:-$ROOT/artifacts/verification/full-native-result.json}"
  python3 scripts/native_lab.py run \
    --resource "iris-chat-rs-five-platform-native-matrix" \
    --result "$result" \
    "${HEALTH_ARGS[@]}" \
    "${ALLOCATION_ARGS[@]}" \
    -- scripts/verify_full_native.sh
}

case "${1:-}" in
  fast)
    run_fast
    ;;
  full)
    if [[ "${IRIS_VERIFY_SKIP_FAST:-0}" != "1" ]]; then
      run_fast
    fi
    run_managed_full
    ;;
  health)
    build_health_args
    python3 scripts/native_lab.py health "${HEALTH_ARGS[@]}"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
