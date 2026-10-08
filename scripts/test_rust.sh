#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${ROOT_DIR}/core"
# Reuse dependencies across the separate manifests without sharing build locks
# with other worktrees. Explicit caller overrides still take precedence.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/core/target}"

# Dependency unit tests are not included by the app's ordinary test command.
cargo test --manifest-path "${ROOT_DIR}/core/vendor/webrtc-ice/Cargo.toml" \
    --locked --lib agent::agent_idle_regression_test

# Device approval has its own build-time relay, separate from SetNostrRelays.
# Keep FFI integration tests local unless the caller supplies a fixture.
if [[ -z "${IRIS_DEVICE_APPROVAL_RELAY_URL:-}" ]]; then
    approval_fixture="$(mktemp -d "${TMPDIR:-/tmp}/iris-rust-approval.XXXXXX")"
    cleanup_approval_fixture() {
        python3 "${ROOT_DIR}/scripts/persistent_local_nostr_relay.py" stop \
            --pid-file "${approval_fixture}/relay.pid" >/dev/null 2>&1 || true
        rm -rf "${approval_fixture}"
    }
    trap cleanup_approval_fixture EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    approval_port="$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
)"
    python3 "${ROOT_DIR}/scripts/persistent_local_nostr_relay.py" start \
        --bind "127.0.0.1:${approval_port}" --host 127.0.0.1 --port "${approval_port}" \
        --pid-file "${approval_fixture}/relay.pid" --log-file "${approval_fixture}/relay.log"
    export IRIS_DEVICE_APPROVAL_RELAY_URL="ws://127.0.0.1:${approval_port}"
fi

# Prefer cargo-nextest when available: it runs test binaries in parallel
# (cargo test runs them serially), which makes a big difference for the
# CLI integration tests. Fall back to cargo test if nextest isn't installed.
for crate in core chat-protocol protocol-ffi; do
    args=(--manifest-path "${ROOT_DIR}/${crate}/Cargo.toml" --locked)
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run --no-fail-fast "${args[@]}"
        cargo test -q --doc "${args[@]}"
    else
        cargo test -q "${args[@]}"
    fi
done
