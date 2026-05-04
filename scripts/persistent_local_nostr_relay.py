#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path


ROOT_DIR = Path(__file__).resolve().parent.parent
CORE_DIR = ROOT_DIR / "core"
DEFAULT_PID_FILE = Path("/tmp/iris-chat-local-relay.pid")
DEFAULT_LOG_FILE = Path("/tmp/iris-chat-local-relay.log")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Start, stop, or inspect a detached local Nostr relay."
    )
    parser.add_argument("command", choices=["start", "stop", "status"])
    parser.add_argument("--bind", default="0.0.0.0:4848")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=4848)
    parser.add_argument("--pid-file", type=Path, default=DEFAULT_PID_FILE)
    parser.add_argument("--log-file", type=Path, default=DEFAULT_LOG_FILE)
    parser.add_argument("--timeout", type=float, default=90)
    return parser.parse_args()


def cargo_target_dir() -> Path:
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        return CORE_DIR / "target"
    target_dir = Path(configured)
    if target_dir.is_absolute():
        return target_dir
    return CORE_DIR / target_dir


def relay_binary() -> Path:
    return cargo_target_dir() / "debug" / "local_nostr_relay"


def build_relay() -> None:
    command = [
        "cargo",
        "build",
        "--manifest-path",
        str(CORE_DIR / "Cargo.toml"),
        "--features",
        "local-relay-bin",
        "--bin",
        "local_nostr_relay",
    ]
    completed = subprocess.run(command)
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)
    if not relay_binary().exists():
        raise SystemExit(f"local_nostr_relay binary not found at {relay_binary()}")


def read_pid(pid_file: Path) -> int | None:
    try:
        return int(pid_file.read_text().strip())
    except (FileNotFoundError, ValueError):
        return None


def process_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def websocket_healthcheck(host: str, port: int, timeout: float = 2) -> bool:
    websocket_key = base64.b64encode(b"ndr-demo-health-check").decode("ascii")
    request = (
        "GET / HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {websocket_key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    ).encode("ascii")
    try:
        with socket.create_connection((host, port), timeout=timeout) as sock:
            sock.settimeout(timeout)
            sock.sendall(request)
            response = sock.recv(256)
    except OSError:
        return False
    return b" 101 " in response or response.startswith(b"HTTP/1.1 101")


def stop_relay(pid_file: Path) -> None:
    pid = read_pid(pid_file)
    if pid is None:
        return
    if process_alive(pid):
        os.kill(pid, signal.SIGTERM)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if not process_alive(pid):
                break
            time.sleep(0.2)
        if process_alive(pid):
            os.kill(pid, signal.SIGKILL)
    try:
        pid_file.unlink()
    except FileNotFoundError:
        pass


def start_relay(args: argparse.Namespace) -> None:
    pid = read_pid(args.pid_file)
    if pid is not None and process_alive(pid):
        if websocket_healthcheck(args.host, args.port):
            print(f"already_running pid={pid} url=ws://{args.host}:{args.port}")
            return
        stop_relay(args.pid_file)

    build_relay()
    args.log_file.parent.mkdir(parents=True, exist_ok=True)
    args.pid_file.parent.mkdir(parents=True, exist_ok=True)
    log_handle = args.log_file.open("ab", buffering=0)
    child = subprocess.Popen(
        [str(relay_binary()), args.bind],
        stdin=subprocess.DEVNULL,
        stdout=log_handle,
        stderr=subprocess.STDOUT,
        close_fds=True,
        start_new_session=True,
    )
    log_handle.close()
    args.pid_file.write_text(f"{child.pid}\n")

    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        if child.poll() is not None:
            break
        if websocket_healthcheck(args.host, args.port):
            print(f"started pid={child.pid} url=ws://{args.host}:{args.port}")
            return
        time.sleep(0.5)

    stop_relay(args.pid_file)
    print(f"failed log={args.log_file}", file=sys.stderr)
    try:
        print(args.log_file.read_text()[-4000:], file=sys.stderr)
    except FileNotFoundError:
        pass
    raise SystemExit(1)


def status_relay(args: argparse.Namespace) -> None:
    pid = read_pid(args.pid_file)
    alive = pid is not None and process_alive(pid)
    healthy = websocket_healthcheck(args.host, args.port)
    print(
        f"pid={pid or ''} alive={str(alive).lower()} "
        f"healthy={str(healthy).lower()} url=ws://{args.host}:{args.port}"
    )
    if not alive or not healthy:
        raise SystemExit(1)


def main() -> int:
    args = parse_args()
    if args.command == "start":
        start_relay(args)
    elif args.command == "stop":
        stop_relay(args.pid_file)
    else:
        status_relay(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
