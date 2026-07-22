#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELPER="$ROOT_DIR/scripts/release_gate_receipt.sh"
TMP_DIR="$(mktemp -d)"
REPO="$TMP_DIR/repo"
WORKTREE="$TMP_DIR/worktree"
trap 'rm -rf "$TMP_DIR"' EXIT

expect_rejected() {
  if "$@" >/dev/null 2>&1; then
    echo "Expected receipt to be rejected: $*" >&2
    exit 1
  fi
}

git init -q "$REPO"
git -C "$REPO" config user.email test@example.invalid
git -C "$REPO" config user.name "Release gate test"
printf 'tracked\n' > "$REPO/tracked.txt"
git -C "$REPO" add tracked.txt
git -C "$REPO" commit -qm initial

receipt_env=(
  env
  IRIS_RELEASE_GATE_ROOT_DIR="$REPO"
  IRIS_RELEASE_GATE_CONFIG="mode=full;platform=test"
  IRIS_RELEASE_GATE_RECEIPT_NOW=100
  IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS=50
)

expect_rejected "${receipt_env[@]}" "$HELPER" check
"${receipt_env[@]}" "$HELPER" write
"${receipt_env[@]}" "$HELPER" check

git -C "$REPO" worktree add -qb receipt-test "$WORKTREE" HEAD
env IRIS_RELEASE_GATE_ROOT_DIR="$WORKTREE" \
  IRIS_RELEASE_GATE_CONFIG="mode=full;platform=test" \
  IRIS_RELEASE_GATE_RECEIPT_NOW=101 \
  IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS=50 \
  "$HELPER" check

expect_rejected env IRIS_RELEASE_GATE_ROOT_DIR="$REPO" \
  IRIS_RELEASE_GATE_CONFIG="mode=local;platform=test" \
  IRIS_RELEASE_GATE_RECEIPT_NOW=101 \
  IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS=50 \
  "$HELPER" check

printf 'dirty\n' > "$REPO/untracked.txt"
expect_rejected "${receipt_env[@]}" "$HELPER" check
rm "$REPO/untracked.txt"

expect_rejected env IRIS_RELEASE_GATE_ROOT_DIR="$REPO" \
  IRIS_RELEASE_GATE_CONFIG="mode=full;platform=test" \
  IRIS_RELEASE_GATE_RECEIPT_NOW=151 \
  IRIS_RELEASE_GATE_RECEIPT_TTL_SECONDS=50 \
  "$HELPER" check

receipt_path="$("${receipt_env[@]}" "$HELPER" path)"
printf 'tampered\n' >> "$receipt_path"
expect_rejected "${receipt_env[@]}" "$HELPER" check
"${receipt_env[@]}" "$HELPER" write

printf 'changed\n' >> "$REPO/tracked.txt"
git -C "$REPO" add tracked.txt
git -C "$REPO" commit -qm changed
expect_rejected "${receipt_env[@]}" "$HELPER" check

echo "Release gate receipt tests passed."
