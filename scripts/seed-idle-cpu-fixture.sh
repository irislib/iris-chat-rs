#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR=""
SECRET_FORMAT="apple"
IRIS_BIN="${IRIS_CHAT_IDLE_CPU_IRIS_BIN:-}"
PEER_HEX="1111111111111111111111111111111111111111111111111111111111111111"
RELAY_PID=""

usage() {
  cat <<'EOF'
usage: scripts/seed-idle-cpu-fixture.sh --data-dir DIR [--secret-format apple|linux|windows]

Creates an isolated real-core account with one direct chat and one self-only
group. Writes fixture.json plus the platform shell's file-backed test secret.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --data-dir) DATA_DIR="$2"; shift 2 ;;
    --secret-format) SECRET_FORMAT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$DATA_DIR" ]] || { usage >&2; exit 2; }
case "$SECRET_FORMAT" in apple|linux|windows) ;; *) echo "Unsupported secret format: $SECRET_FORMAT" >&2; exit 2 ;; esac
TARGET_DIR="$(cargo metadata --manifest-path "$ROOT/core/Cargo.toml" --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

if [[ -z "$IRIS_BIN" ]]; then
  cargo build \
    --manifest-path "$ROOT/core/Cargo.toml" \
    --features local-relay-bin \
    --bin iris \
    --bin local_nostr_relay
  IRIS_BIN="$TARGET_DIR/debug/iris"
  [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]] && IRIS_BIN="${IRIS_BIN}.exe"
fi
[[ -x "$IRIS_BIN" || -f "$IRIS_BIN" ]] || { echo "Missing iris CLI: $IRIS_BIN" >&2; exit 1; }

rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"

cleanup() {
  if [[ -n "$RELAY_PID" ]]; then
    kill "$RELAY_PID" >/dev/null 2>&1 || true
    wait "$RELAY_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

RELAY_PORT="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
RELAY_BIN="$TARGET_DIR/debug/local_nostr_relay"
if [[ ! -x "$RELAY_BIN" ]]; then
  cargo build \
    --manifest-path "$ROOT/core/Cargo.toml" \
    --features local-relay-bin \
    --bin local_nostr_relay
fi
"$RELAY_BIN" "127.0.0.1:$RELAY_PORT" >"$DATA_DIR/relay.log" 2>&1 &
RELAY_PID=$!
python3 - "$RELAY_PORT" <<'PY'
import socket
import sys
import time

port = int(sys.argv[1])
deadline = time.monotonic() + 60
while time.monotonic() < deadline:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            break
    except OSError:
        time.sleep(0.2)
else:
    raise SystemExit("local fixture message server did not start")
PY

run_iris() {
  "$IRIS_BIN" --json --data-dir "$DATA_DIR" "$@"
}

run_iris relay set "ws://127.0.0.1:$RELAY_PORT" >"$DATA_DIR/relay.json"
run_iris account create --name "Idle CPU Alice" >"$DATA_DIR/account.json.out"
run_iris chat create "$PEER_HEX" >"$DATA_DIR/direct.json"
run_iris group create "Idle CPU group" >"$DATA_DIR/group.json"
run_iris relay reset >"$DATA_DIR/relay-reset.json"
run_iris state >"$DATA_DIR/state.json"

python3 - "$DATA_DIR" "$SECRET_FORMAT" <<'PY'
import json
import os
import stat
import sys
from pathlib import Path

data_dir = Path(sys.argv[1])
secret_format = sys.argv[2]

def envelope(path):
    payload = json.loads(path.read_text(encoding="utf-8"))
    if payload.get("status") != "ok":
        raise SystemExit(f"fixture command failed: {path}: {payload}")
    return payload["data"]

state = envelope(data_dir / "state.json")
account = state.get("account")
chats = state.get("chats") or []
direct_count = sum(chat.get("kind") == "direct" for chat in chats)
group_count = sum(chat.get("kind") == "group" for chat in chats)
if not account or direct_count < 1 or group_count < 1:
    raise SystemExit(
        f"fixture did not reach required state: account={bool(account)} "
        f"direct={direct_count} group={group_count}"
    )

cli_secret_path = data_dir / "cli-account.json"
cli_secret = json.loads(cli_secret_path.read_text(encoding="utf-8"))
if secret_format == "apple":
    secret_path = data_dir / "account-secret.json"
    secret = {
        "ownerNsec": cli_secret.get("owner_nsec"),
        "ownerPubkeyHex": cli_secret["owner_pubkey_hex"],
        "deviceNsec": cli_secret["device_nsec"],
    }
elif secret_format == "windows":
    secret_path = data_dir / "account-secret.json"
    secret = {
        "OwnerNsec": cli_secret.get("owner_nsec"),
        "OwnerPubkeyHex": cli_secret["owner_pubkey_hex"],
        "DeviceNsec": cli_secret["device_nsec"],
    }
else:
    secret_path = data_dir / "account.json"
    secret = cli_secret

secret_path.write_text(json.dumps(secret, separators=(",", ":")) + "\n", encoding="utf-8")
if os.name != "nt":
    secret_path.chmod(stat.S_IRUSR | stat.S_IWUSR)

fixture = {
    "loggedIn": True,
    "directChatCount": direct_count,
    "groupChatCount": group_count,
    "accountUserId": account.get("user_id", ""),
    "dataDir": str(data_dir),
}
(data_dir / "fixture.json").write_text(
    json.dumps(fixture, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(fixture, sort_keys=True))
PY
