#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-reliability-reuse.XXXXXX")"
OUTPUT="$RUN_DIR/output.log"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

"$ROOT/scripts/e2e_reliability_lab.sh" \
  --tier daily \
  --tier soak \
  --dry-run \
  --run-dir "$RUN_DIR" \
  --android-avds "Test AVD A|Test AVD B" \
  --ios-simulators "Test iPhone A|Test iPhone B" >"$OUTPUT"

grep -q '^reuse_local_builds=1$' "$RUN_DIR/manifest.env"
relay_port="$(sed -n 's/^reliability_relay_port=//p' "$RUN_DIR/manifest.env")"
[[ "$relay_port" =~ ^[0-9]+$ ]]

command_count="$(grep -c '^+' "$OUTPUT")"
[[ "$command_count" -eq 10 ]]
first_command="$(grep '^+' "$OUTPUT" | sed -n '1p')"
second_command="$(grep '^+' "$OUTPUT" | sed -n '2p')"

# The first iOS-only and Android-only flows populate one build per platform.
[[ "$first_command" != *"--skip-build"* ]]
[[ "$second_command" != *"--skip-build"* ]]
if grep '^+' "$OUTPUT" | tail -n +3 | grep -vq -- '--skip-build'; then
  echo "a reused reliability flow did not pass --skip-build" >&2
  exit 1
fi

[[ "$first_command" == *"--relay-url ws://127.0.0.1:${relay_port}"* ]]
[[ "$second_command" == *"--relay-url ws://10.0.2.2:${relay_port}"* ]]
grep -q -- "--android-relay-url ws://10.0.2.2:${relay_port} --skip-build" "$OUTPUT"

echo "reliability build reuse harness passed"
