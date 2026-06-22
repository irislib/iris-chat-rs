#!/usr/bin/env bash

# Shared helpers for small groups of independent checks. Output is captured per
# step and replayed after completion so parallel jobs do not interleave logs.

PARALLEL_STEP_TMP_DIR=""
PARALLEL_STEP_LABELS=()
PARALLEL_STEP_LOGS=()
PARALLEL_STEP_PIDS=()

parallel_steps_reset() {
  if [[ -n "${PARALLEL_STEP_TMP_DIR:-}" && -d "$PARALLEL_STEP_TMP_DIR" ]]; then
    rm -rf "$PARALLEL_STEP_TMP_DIR"
  fi
  PARALLEL_STEP_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-parallel.XXXXXX")"
  PARALLEL_STEP_LABELS=()
  PARALLEL_STEP_LOGS=()
  PARALLEL_STEP_PIDS=()
}

parallel_steps_cleanup() {
  if [[ -n "${PARALLEL_STEP_TMP_DIR:-}" && -d "$PARALLEL_STEP_TMP_DIR" ]]; then
    rm -rf "$PARALLEL_STEP_TMP_DIR"
  fi
  PARALLEL_STEP_TMP_DIR=""
  PARALLEL_STEP_LABELS=()
  PARALLEL_STEP_LOGS=()
  PARALLEL_STEP_PIDS=()
}

parallel_steps_abort() {
  local pid
  for pid in "${PARALLEL_STEP_PIDS[@]:-}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  parallel_steps_cleanup
}

parallel_step_start() {
  local label="$1"
  shift
  if [[ $# -eq 0 ]]; then
    echo "parallel_step_start requires a command for: $label" >&2
    return 2
  fi
  if [[ -z "${PARALLEL_STEP_TMP_DIR:-}" ]]; then
    parallel_steps_reset
  fi

  local index log
  index="${#PARALLEL_STEP_PIDS[@]}"
  log="$PARALLEL_STEP_TMP_DIR/${index}.log"
  echo "=== ${label} (background) ==="
  (
    set -Eeuo pipefail
    "$@"
  ) >"$log" 2>&1 &

  PARALLEL_STEP_LABELS+=("$label")
  PARALLEL_STEP_LOGS+=("$log")
  PARALLEL_STEP_PIDS+=("$!")
}

parallel_step_wait() {
  local failed=0
  local status=0
  local i

  for i in "${!PARALLEL_STEP_PIDS[@]}"; do
    status=0
    wait "${PARALLEL_STEP_PIDS[$i]}" || status=$?
    echo
    if [[ "$status" -eq 0 ]]; then
      echo "=== ${PARALLEL_STEP_LABELS[$i]} passed ==="
      cat "${PARALLEL_STEP_LOGS[$i]}"
    else
      echo "=== ${PARALLEL_STEP_LABELS[$i]} failed (exit ${status}) ===" >&2
      cat "${PARALLEL_STEP_LOGS[$i]}" >&2
      failed=1
    fi
  done

  parallel_steps_cleanup
  return "$failed"
}

run_parallel_steps() {
  parallel_steps_reset

  local label
  local -a command
  while [[ $# -gt 0 ]]; do
    label="$1"
    shift
    command=()
    while [[ $# -gt 0 ]]; do
      if [[ "$1" == ":::" ]]; then
        shift
        break
      fi
      command+=("$1")
      shift
    done
    if [[ "${#command[@]}" -eq 0 ]]; then
      echo "run_parallel_steps requires a command for: $label" >&2
      parallel_steps_abort
      return 2
    fi
    parallel_step_start "$label" "${command[@]}"
  done

  parallel_step_wait
}
