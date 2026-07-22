#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARALLEL_STEPS="$ROOT/scripts/parallel_steps.sh"
TEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/iris-parallel-harness.XXXXXX")"
CONTROLLER_PIDS=()

cleanup() {
  local pid_file pid

  for pid in "${CONTROLLER_PIDS[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done

  # Clean up grandchildren as well when testing a deliberately broken helper.
  for pid_file in "$TEST_TMP"/*.pid "$TEST_TMP"/*/*.pid; do
    [[ -f "$pid_file" ]] || continue
    pid="$(sed -n '1p' "$pid_file")"
    if [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done

  rm -rf "$TEST_TMP"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  echo "parallel steps harness failed: $*" >&2
  exit 1
}

assert_contains() {
  local path="$1"
  local text="$2"
  grep -Fq -- "$text" "$path" || fail "missing '$text' in $path"
}

assert_line_once() {
  local path="$1"
  local text="$2"
  local count
  count="$(grep -Fxc -- "$text" "$path" || true)"
  [[ "$count" -eq 1 ]] || fail "expected one exact '$text' line in $path, found $count"
}

pid_stopped() {
  ! kill -0 "$1" 2>/dev/null
}

file_exists() {
  [[ -e "$1" ]]
}

wait_until() {
  local attempts="$1"
  shift
  local i

  for ((i = 0; i < attempts; i++)); do
    if "$@"; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

run_parallel_log_test() {
  local case_dir="$TEST_TMP/parallel-log"
  local output="$case_dir/output.log"
  local helper_tmp
  local alpha_one alpha_two alpha_three
  local beta_one beta_two beta_three
  mkdir -p "$case_dir"

  (
    # shellcheck disable=SC1090
    source "$PARALLEL_STEPS"

    parallel_step_start "alpha fidelity" bash -c '
      set -Eeuo pipefail
      touch "$1/alpha.ready"
      attempts=0
      while [[ ! -e "$1/beta.ready" ]]; do
        attempts=$((attempts + 1))
        if [[ "$attempts" -ge 100 ]]; then
          echo "alpha did not overlap beta" >&2
          exit 90
        fi
        sleep 0.02
      done
      echo "ALPHA stdout 1"
      echo "ALPHA stderr 2" >&2
      echo "ALPHA stdout 3"
    ' _ "$case_dir"

    parallel_step_start "beta fidelity" bash -c '
      set -Eeuo pipefail
      touch "$1/beta.ready"
      attempts=0
      while [[ ! -e "$1/alpha.ready" ]]; do
        attempts=$((attempts + 1))
        if [[ "$attempts" -ge 100 ]]; then
          echo "beta did not overlap alpha" >&2
          exit 91
        fi
        sleep 0.02
      done
      echo "BETA stdout 1"
      echo "BETA stderr 2" >&2
      echo "BETA stdout 3"
    ' _ "$case_dir"

    printf '%s\n' "$PARALLEL_STEP_TMP_DIR" > "$case_dir/helper-tmp"
    helper_tmp="$PARALLEL_STEP_TMP_DIR"
    parallel_step_wait

    [[ -z "${PARALLEL_STEP_TMP_DIR:-}" ]]
    [[ ! -e "$helper_tmp" ]]
  ) >"$output" 2>&1 || {
    sed -n '1,240p' "$output" >&2
    fail "overlap/log scenario returned failure"
  }

  helper_tmp="$(sed -n '1p' "$case_dir/helper-tmp")"
  [[ ! -e "$helper_tmp" ]] || fail "successful wait leaked temporary logs: $helper_tmp"

  assert_contains "$output" "=== alpha fidelity passed ==="
  assert_contains "$output" "=== beta fidelity passed ==="
  assert_line_once "$output" "ALPHA stdout 1"
  assert_line_once "$output" "ALPHA stderr 2"
  assert_line_once "$output" "ALPHA stdout 3"
  assert_line_once "$output" "BETA stdout 1"
  assert_line_once "$output" "BETA stderr 2"
  assert_line_once "$output" "BETA stdout 3"

  alpha_one="$(grep -nFx -- "ALPHA stdout 1" "$output" | cut -d: -f1)"
  alpha_two="$(grep -nFx -- "ALPHA stderr 2" "$output" | cut -d: -f1)"
  alpha_three="$(grep -nFx -- "ALPHA stdout 3" "$output" | cut -d: -f1)"
  beta_one="$(grep -nFx -- "BETA stdout 1" "$output" | cut -d: -f1)"
  beta_two="$(grep -nFx -- "BETA stderr 2" "$output" | cut -d: -f1)"
  beta_three="$(grep -nFx -- "BETA stdout 3" "$output" | cut -d: -f1)"

  [[ "$alpha_two" -eq $((alpha_one + 1)) && "$alpha_three" -eq $((alpha_two + 1)) ]] \
    || fail "alpha stdout/stderr was interleaved with another step"
  [[ "$beta_two" -eq $((beta_one + 1)) && "$beta_three" -eq $((beta_two + 1)) ]] \
    || fail "beta stdout/stderr was interleaved with another step"
}

run_fail_fast_test() {
  local case_dir="$TEST_TMP/fail-fast"
  local output="$case_dir/output.log"
  local controller sibling_pid helper_tmp status
  mkdir -p "$case_dir"

  (
    # shellcheck disable=SC1090
    source "$PARALLEL_STEPS"

    # Start the slow job first. A launch-order wait would block here forever and
    # never notice that the second job has already failed.
    parallel_step_start "slow sibling" bash -c '
      set -Eeuo pipefail
      case_dir="$1"
      on_signal() {
        echo "slow sibling terminated" > "$case_dir/sibling-terminated"
        exit 143
      }
      trap on_signal TERM INT
      echo "$$" > "$case_dir/sibling.pid"
      echo "slow sibling started"
      while :; do sleep 0.05; done
    ' _ "$case_dir"

    parallel_step_start "quick failure" bash -c '
      set -Eeuo pipefail
      sleep 0.15
      echo "quick failure stdout"
      echo "quick failure stderr" >&2
      exit 23
    '

    printf '%s\n' "$PARALLEL_STEP_TMP_DIR" > "$case_dir/helper-tmp"
    parallel_step_wait
  ) >"$output" 2>&1 &
  controller=$!
  CONTROLLER_PIDS+=("$controller")

  wait_until 40 file_exists "$case_dir/sibling.pid" \
    || fail "slow sibling did not start"

  if ! wait_until 60 pid_stopped "$controller"; then
    sed -n '1,240p' "$output" >&2
    kill -TERM "$controller" 2>/dev/null || true
    fail "parallel wait did not fail promptly after a later step failed"
  fi

  set +e
  wait "$controller"
  status=$?
  set -e
  [[ "$status" -ne 0 ]] || fail "parallel wait hid the failing step"

  assert_contains "$output" "=== quick failure failed (exit 23) ==="
  assert_line_once "$output" "quick failure stdout"
  assert_line_once "$output" "quick failure stderr"

  wait_until 40 file_exists "$case_dir/sibling-terminated" \
    || fail "fail-fast did not ask the slow sibling to terminate"
  sibling_pid="$(sed -n '1p' "$case_dir/sibling.pid")"
  wait_until 40 pid_stopped "$sibling_pid" \
    || fail "slow sibling process $sibling_pid survived fail-fast cleanup"

  helper_tmp="$(sed -n '1p' "$case_dir/helper-tmp")"
  [[ ! -e "$helper_tmp" ]] || fail "failed wait leaked temporary logs: $helper_tmp"
}

run_signal_cleanup_test() {
  local case_dir="$TEST_TMP/signal-cleanup"
  local output="$case_dir/output.log"
  local controller worker_pid helper_tmp status
  mkdir -p "$case_dir"

  (
    # shellcheck disable=SC1090
    source "$PARALLEL_STEPS"

    parallel_step_start "signal worker" bash -c '
      set -Eeuo pipefail
      case_dir="$1"
      on_signal() {
        echo "signal worker terminated" > "$case_dir/worker-terminated"
        exit 143
      }
      trap on_signal TERM INT
      echo "$$" > "$case_dir/worker.pid"
      while :; do sleep 0.05; done
    ' _ "$case_dir"

    printf '%s\n' "$PARALLEL_STEP_TMP_DIR" > "$case_dir/helper-tmp"
    touch "$case_dir/controller-ready"
    while :; do sleep 0.05; done
  ) >"$output" 2>&1 &
  controller=$!
  CONTROLLER_PIDS+=("$controller")

  wait_until 40 file_exists "$case_dir/controller-ready" \
    || fail "signal cleanup controller did not become ready"
  wait_until 40 file_exists "$case_dir/worker.pid" \
    || fail "signal cleanup worker did not start"

  kill -TERM "$controller"
  wait_until 40 pid_stopped "$controller" \
    || fail "controller ignored TERM"

  set +e
  wait "$controller"
  status=$?
  set -e
  [[ "$status" -ne 0 ]] || fail "TERM unexpectedly reported success"

  wait_until 40 file_exists "$case_dir/worker-terminated" \
    || fail "TERM did not propagate cleanup to the background worker"
  worker_pid="$(sed -n '1p' "$case_dir/worker.pid")"
  wait_until 40 pid_stopped "$worker_pid" \
    || fail "background worker $worker_pid survived controller TERM"

  helper_tmp="$(sed -n '1p' "$case_dir/helper-tmp")"
  [[ ! -e "$helper_tmp" ]] || fail "TERM leaked temporary logs: $helper_tmp"
}

[[ -r "$PARALLEL_STEPS" ]] || fail "missing $PARALLEL_STEPS"

run_parallel_log_test
run_fail_fast_test
run_signal_cleanup_test

echo "parallel steps harness passed"
