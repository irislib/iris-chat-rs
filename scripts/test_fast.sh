#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/parallel_steps.sh"

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

run_step "Rust panic/unwrap lint" "${ROOT_DIR}/scripts/check-rust-panics" || lint_status=$?
parallel_step_wait || static_status=$?
if [[ "$lint_status" -ne 0 || "$static_status" -ne 0 ]]; then
  exit 1
fi

run_step "Rust tests" "${ROOT_DIR}/scripts/test_rust.sh"

if [[ "${IRIS_FAST_RUN_SOAK:-0}" == "1" ]]; then
  run_step "serial Rust soak" \
    "${ROOT_DIR}/scripts/local_relay_scenario_soak.sh" \
    --iterations "${IRIS_FAST_SOAK_ITERATIONS:-1}"
fi

run_step "Android Kotlin compile" \
  bash -c 'cd "$1" && ./gradlew :app:compileDebugKotlin :app:compileDebugAndroidTestKotlin' \
  _ "${ROOT_DIR}/android"
run_step "iOS tests" "${ROOT_DIR}/scripts/ios-build" ios-test
