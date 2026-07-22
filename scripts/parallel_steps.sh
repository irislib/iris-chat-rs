#!/usr/bin/env bash

# Shared helpers for small groups of independent checks. Output is captured per
# step and replayed after completion so parallel jobs do not interleave logs.

PARALLEL_STEP_TMP_DIR=""
PARALLEL_STEP_LABELS=()
PARALLEL_STEP_LOGS=()
PARALLEL_STEP_PIDS=()
PARALLEL_STEP_DONE=()
PARALLEL_STEP_STATUSES=()

parallel_steps_reset() {
  if [[ -n "${PARALLEL_STEP_TMP_DIR:-}" && -d "$PARALLEL_STEP_TMP_DIR" ]]; then
    rm -rf "$PARALLEL_STEP_TMP_DIR"
  fi
  PARALLEL_STEP_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-parallel.XXXXXX")"
  PARALLEL_STEP_LABELS=()
  PARALLEL_STEP_LOGS=()
  PARALLEL_STEP_PIDS=()
  PARALLEL_STEP_DONE=()
  PARALLEL_STEP_STATUSES=()
}

parallel_steps_cleanup() {
  if [[ -n "${PARALLEL_STEP_TMP_DIR:-}" && -d "$PARALLEL_STEP_TMP_DIR" ]]; then
    rm -rf "$PARALLEL_STEP_TMP_DIR"
  fi
  PARALLEL_STEP_TMP_DIR=""
  PARALLEL_STEP_LABELS=()
  PARALLEL_STEP_LOGS=()
  PARALLEL_STEP_PIDS=()
  PARALLEL_STEP_DONE=()
  PARALLEL_STEP_STATUSES=()
}

parallel_step_kill_tree() {
  local pid="$1"
  local signal="${2:-TERM}"
  local child
  local children=""

  if command -v pgrep >/dev/null 2>&1; then
    children="$(pgrep -P "$pid" 2>/dev/null || true)"
  fi
  # Notify the owning shell first so its cleanup trap is pending before a
  # foreground child exits. Bash otherwise may leave the shell through `set -e`
  # without running the command's TERM trap.
  kill "-${signal}" "$pid" 2>/dev/null || true
  [[ "$signal" != "TERM" ]] || sleep 0.02
  for child in $children; do
    parallel_step_kill_tree "$child" "$signal"
  done
}

parallel_steps_stop_pending() {
  local i pid attempt

  for i in "${!PARALLEL_STEP_PIDS[@]}"; do
    [[ "${PARALLEL_STEP_DONE[$i]:-0}" == "1" ]] && continue
    pid="${PARALLEL_STEP_PIDS[$i]}"
    parallel_step_kill_tree "$pid" TERM
  done

  for attempt in $(seq 1 40); do
    local running=0
    for i in "${!PARALLEL_STEP_PIDS[@]}"; do
      [[ "${PARALLEL_STEP_DONE[$i]:-0}" == "1" ]] && continue
      if kill -0 "${PARALLEL_STEP_PIDS[$i]}" 2>/dev/null; then
        running=1
      fi
    done
    [[ "$running" -eq 0 ]] && break
    sleep 0.05
  done

  for i in "${!PARALLEL_STEP_PIDS[@]}"; do
    [[ "${PARALLEL_STEP_DONE[$i]:-0}" == "1" ]] && continue
    pid="${PARALLEL_STEP_PIDS[$i]}"
    if kill -0 "$pid" 2>/dev/null; then
      parallel_step_kill_tree "$pid" KILL
    fi
    wait "$pid" 2>/dev/null || true
    PARALLEL_STEP_DONE[$i]=1
  done
}

parallel_steps_abort() {
  parallel_steps_stop_pending
  parallel_steps_cleanup
}

parallel_steps_on_exit() {
  local status=$?
  trap - EXIT
  parallel_steps_abort
  exit "$status"
}

parallel_steps_on_signal() {
  local signal="$1"
  local status="$2"
  trap - "$signal"
  parallel_steps_abort
  exit "$status"
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
    # The controller owns sibling cleanup. A worker must not inherit its EXIT
    # trap and terminate jobs that were launched before this one.
    trap - EXIT INT TERM
    set -Eeuo pipefail
    "$@"
  ) >"$log" 2>&1 &

  PARALLEL_STEP_LABELS+=("$label")
  PARALLEL_STEP_LOGS+=("$log")
  PARALLEL_STEP_PIDS+=("$!")
  PARALLEL_STEP_DONE+=(0)
  PARALLEL_STEP_STATUSES+=(0)
}

parallel_step_wait() {
  local count="${#PARALLEL_STEP_PIDS[@]}"
  local remaining="$count"
  local status=0
  local i

  while [[ "$remaining" -gt 0 ]]; do
    for i in "${!PARALLEL_STEP_PIDS[@]}"; do
      [[ "${PARALLEL_STEP_DONE[$i]:-0}" == "1" ]] && continue
      if kill -0 "${PARALLEL_STEP_PIDS[$i]}" 2>/dev/null; then
        continue
      fi

      status=0
      wait "${PARALLEL_STEP_PIDS[$i]}" || status=$?
      PARALLEL_STEP_DONE[$i]=1
      PARALLEL_STEP_STATUSES[$i]="$status"
      remaining=$((remaining - 1))

      if [[ "$status" -ne 0 ]]; then
        echo >&2
        echo "=== ${PARALLEL_STEP_LABELS[$i]} failed (exit ${status}) ===" >&2
        cat "${PARALLEL_STEP_LOGS[$i]}" >&2
        parallel_steps_stop_pending
        parallel_steps_cleanup
        return 1
      fi
    done
    [[ "$remaining" -eq 0 ]] || sleep 0.05
  done

  for i in "${!PARALLEL_STEP_PIDS[@]}"; do
    echo
    echo "=== ${PARALLEL_STEP_LABELS[$i]} passed ==="
    cat "${PARALLEL_STEP_LOGS[$i]}"
  done

  parallel_steps_cleanup
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

# The helper owns only jobs started through parallel_step_start. Installing
# cleanup here makes every caller safe when interrupted or when it exits early.
trap parallel_steps_on_exit EXIT
trap 'parallel_steps_on_signal INT 130' INT
trap 'parallel_steps_on_signal TERM 143' TERM
