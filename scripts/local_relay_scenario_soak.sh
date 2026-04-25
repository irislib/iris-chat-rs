#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ITERATIONS="${ITERATIONS:-100}"
RUST_DIR="${ROOT_DIR}/core"

usage() {
  cat <<EOF
Usage: scripts/local_relay_scenario_soak.sh [--iterations N]

Runs the core test suite repeatedly.

Options:
  --iterations N   Number of full scenario-suite passes. Default: ${ITERATIONS}
  -h, --help       Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --iterations)
      ITERATIONS="$2"
      shift 2
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

for ((iteration = 1; iteration <= ITERATIONS; iteration++)); do
  echo "=== local relay soak iteration ${iteration}/${ITERATIONS} ==="
  (
    cd "${RUST_DIR}" &&
      cargo test -- --nocapture --test-threads=1
  )
done

echo "Local relay scenario soak passed (${ITERATIONS} iterations)"
