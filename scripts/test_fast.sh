#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/parallel_steps.sh"

RUN_NATIVE=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --core-only)
      RUN_NATIVE=0
      shift
      ;;
    -h|--help)
      echo "usage: scripts/test_fast.sh [--core-only]"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

run_step() {
  local name="$1"
  shift
  echo
  echo "=== ${name} ==="
  "$@"
}

static_status=0
lint_status=0

parallel_step_start "palette parity" "${ROOT_DIR}/scripts/check-palettes"
parallel_step_start "brand accent text guard" "${ROOT_DIR}/scripts/check-no-accent-text"
parallel_step_start "source file size guard" "${ROOT_DIR}/scripts/check-source-file-sizes"
parallel_step_start "verification tier contract" "${ROOT_DIR}/scripts/check_verification_tiers.sh"
parallel_step_start "parallel step harness" "${ROOT_DIR}/scripts/test-parallel-steps-harness.sh"
parallel_step_start "reliability build reuse harness" \
  "${ROOT_DIR}/scripts/test-reliability-build-reuse-harness.sh"
parallel_step_start "idle CPU gate harness" "${ROOT_DIR}/scripts/test-idle-cpu-gate-harness.sh"
parallel_step_start "iOS simulator recovery harness" "${ROOT_DIR}/scripts/test-ios-simulator-recovery.sh"
parallel_step_start "iOS cloneable simulator selection harness" \
  "${ROOT_DIR}/scripts/test-ios-cloneable-simulator-selection.sh"
parallel_step_start "iOS harness retry contract" python3 "${ROOT_DIR}/scripts/test_ios_harness_retry_contract.py"
parallel_step_start "mobile relay AVD selection harness" "${ROOT_DIR}/scripts/test-mobile-relay-common.sh"
parallel_step_start "mobile push FCM selection harness" \
  "${ROOT_DIR}/scripts/test-mobile-push-server-e2e.sh"

run_step "Rust panic/unwrap lint" "${ROOT_DIR}/scripts/check-rust-panics" || lint_status=$?
parallel_step_wait || static_status=$?
if [[ "$lint_status" -ne 0 || "$static_status" -ne 0 ]]; then
  exit 1
fi

run_rust_and_ios_tests() {
  run_step "Rust tests" "${ROOT_DIR}/scripts/test_rust.sh"

  run_step "iOS tests" "${ROOT_DIR}/scripts/ios-build" ios-test
}

if [[ "$RUN_NATIVE" == "1" ]]; then
  # Android uses its worktree-specific Cargo target by default, while Rust and
  # iOS intentionally share core/target and therefore remain in one lane.
  parallel_step_start "Android Kotlin compile" \
    bash -c 'cd "$1" && ./gradlew :app:compileDebugKotlin :app:compileDebugAndroidTestKotlin' \
    _ "${ROOT_DIR}/android"
  parallel_step_start "Rust and iOS tests" run_rust_and_ios_tests
  parallel_step_wait
else
  run_step "Rust tests" "${ROOT_DIR}/scripts/test_rust.sh"
fi

if [[ "${IRIS_FAST_RUN_SOAK:-0}" == "1" ]]; then
  run_step "serial Rust soak" \
    "${ROOT_DIR}/scripts/local_relay_scenario_soak.sh" \
    --iterations "${IRIS_FAST_SOAK_ITERATIONS:-1}"
fi

if [[ "$RUN_NATIVE" != "1" ]]; then
  echo
  echo "=== native checks deferred to full verification ==="
fi
