#!/usr/bin/env bash
# Proves the production call lifecycle and compressed-media datagrams with all non-loopback
# networking denied by macOS, not merely with an empty message-server list.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -s)" != Darwin ]] || ! command -v sandbox-exec >/dev/null; then
    echo 'Offline network isolation requires macOS sandbox-exec on this runner.' >&2
    exit 75
fi
BUILD_RESULT="$(mktemp)"
trap 'rm -f "$BUILD_RESULT"' EXIT
cargo test --manifest-path "$ROOT_DIR/core/Cargo.toml" --locked --lib --no-run --message-format=json > "$BUILD_RESULT"
TEST_BINARY="$(python3 - "$BUILD_RESULT" <<'PY'
import json,sys
for line in open(sys.argv[1]):
    artifact=json.loads(line)
    if artifact.get('reason')=='compiler-artifact' and artifact.get('profile',{}).get('test') and artifact.get('executable'):
        print(artifact['executable'])
        break
else:
    raise SystemExit('Rust test binary was not produced')
PY
)"
ISOLATION='(version 1)(allow default)(deny network*)(allow network-outbound (remote ip "localhost:*"))(allow network-inbound (local ip "localhost:*"))(allow network-bind (local ip "localhost:*"))'
sandbox-exec -p "$ISOLATION" python3 - <<'PY'
import socket
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sender, socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as receiver:
    sender.bind(('127.0.0.1',0)); receiver.bind(('127.0.0.1',0)); receiver.settimeout(1)
    sender.sendto(b'local-network-test',receiver.getsockname())
    assert receiver.recvfrom(64)[0]==b'local-network-test', 'Local networking was blocked'
with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as connection:
    try:
        connection.sendto(b'blocked-network-test',('192.0.2.1',9))
    except PermissionError:
        print('Verified: nonlocal networking is denied.')
    else:
        raise SystemExit('Network isolation did not deny external traffic')
PY
sandbox-exec -p "$ISOLATION" "$TEST_BINARY" --exact core::tests::calls_e2e_without_internet_over_local_fips_udp --nocapture
