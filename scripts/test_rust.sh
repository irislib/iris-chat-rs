#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${ROOT_DIR}/core"
# Reuse dependencies across the separate manifests without sharing build locks
# with other worktrees. Explicit caller overrides still take precedence.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/core/target}"

# Prefer cargo-nextest when available: it runs test binaries in parallel
# (cargo test runs them serially), which makes a big difference for the
# CLI integration tests. Fall back to cargo test if nextest isn't installed.
for crate in core chat-protocol protocol-ffi; do
    args=(--manifest-path "${ROOT_DIR}/${crate}/Cargo.toml" --locked)
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run "${args[@]}"
        cargo test -q --doc "${args[@]}"
    else
        cargo test -q "${args[@]}"
    fi
done
