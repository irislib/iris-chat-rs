#!/usr/bin/env python3
"""
Subscribe to a local Nostr relay via WebSocket, wait for matching events,
and print their event JSON. Used by smoke scripts that
need to capture the encrypted kind:1060 wrapper a peer publishes — the
exact payload the notification server would forward to FCM/APNs.

Stdlib only (no external `websockets` dep) so the harness can pick it
up without a venv. Implements the bare minimum of RFC 6455 (text frames
+ ping/pong) + Nostr REQ/EVENT framing.

Usage:
  capture_relay_event.py --relay ws://192.168.178.81:4848 \
                         --kinds 1060 \
                         --p-tag <hex_recipient> \
                         --timeout-secs 60
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time
from urllib.parse import urlparse


def encode_frame(payload: bytes) -> bytes:
    # Single-fragment text frame, masked (clients always mask).
    header = bytes([0x81])
    mask_bit = 0x80
    length = len(payload)
    if length < 126:
        header += bytes([mask_bit | length])
    elif length < 65536:
        header += bytes([mask_bit | 126]) + struct.pack(">H", length)
    else:
        header += bytes([mask_bit | 127]) + struct.pack(">Q", length)
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return header + mask + masked


def read_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed mid-read")
        buf += chunk
    return buf


def read_frame(sock: socket.socket) -> tuple[int, bytes]:
    header = read_exact(sock, 2)
    opcode = header[0] & 0x0F
    masked = bool(header[1] & 0x80)
    length = header[1] & 0x7F
    if length == 126:
        length = struct.unpack(">H", read_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack(">Q", read_exact(sock, 8))[0]
    mask = read_exact(sock, 4) if masked else b""
    payload = read_exact(sock, length)
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return opcode, payload


def open_websocket(url: str, timeout: float) -> socket.socket:
    parsed = urlparse(url)
    if parsed.scheme not in ("ws", "wss"):
        raise SystemExit(f"unsupported scheme {parsed.scheme}")
    if parsed.scheme == "wss":
        raise SystemExit("wss:// not supported by this minimal helper")
    host = parsed.hostname or "127.0.0.1"
    port = parsed.port or 80
    path = parsed.path or "/"
    sock = socket.create_connection((host, port), timeout=timeout)
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    handshake = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(handshake.encode())
    response = b""
    while b"\r\n\r\n" not in response:
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("relay closed during handshake")
        response += chunk
    if b" 101 " not in response.split(b"\r\n", 1)[0]:
        raise SystemExit(f"relay refused upgrade: {response.splitlines()[0].decode(errors='replace')}")
    return sock


def main() -> int:
    parser = argparse.ArgumentParser(description="Capture a Nostr event from a local relay")
    parser.add_argument("--relay", required=True, help="ws://host:port")
    parser.add_argument("--kinds", default="1060", help="comma-separated event kinds")
    parser.add_argument("--p-tag", help="filter events tagged with #p=<hex>")
    parser.add_argument("--author", help="filter events authored by <hex> (comma- or pipe-separated)")
    parser.add_argument("--since-secs", type=int, default=0, help="only consider events created_at >= now-since")
    parser.add_argument("--timeout-secs", type=float, default=60.0)
    parser.add_argument("--count", type=int, default=1, help="number of matching events to capture")
    parser.add_argument(
        "--format",
        choices=("single", "jsonl", "array"),
        default="single",
        help="output format; single requires --count=1",
    )
    args = parser.parse_args()
    if args.count < 1:
        raise SystemExit("--count must be >= 1")
    if args.format == "single" and args.count != 1:
        raise SystemExit("--format single requires --count=1")

    sock = open_websocket(args.relay, timeout=10.0)
    sock.settimeout(args.timeout_secs)

    sub_id = "capture-" + base64.urlsafe_b64encode(os.urandom(6)).decode().rstrip("=")
    filter_ = {"kinds": [int(k) for k in args.kinds.split(",") if k.strip()]}
    if args.p_tag:
        filter_["#p"] = [args.p_tag.lower()]
    if args.author:
        authors = [
            value.strip().lower()
            for value in args.author.replace("|", ",").split(",")
            if value.strip()
        ]
        if authors:
            filter_["authors"] = authors
    if args.since_secs:
        filter_["since"] = int(time.time()) - args.since_secs
    req = json.dumps(["REQ", sub_id, filter_])
    sock.sendall(encode_frame(req.encode()))

    events = []
    deadline = time.time() + args.timeout_secs
    while time.time() < deadline:
        try:
            opcode, payload = read_frame(sock)
        except (socket.timeout, ConnectionError):
            break
        if opcode == 0x9:  # ping
            pong = bytes([0x8A, 0x80]) + os.urandom(4)
            sock.sendall(pong)
            continue
        if opcode != 0x1:
            continue
        try:
            decoded = json.loads(payload.decode())
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        if not isinstance(decoded, list) or not decoded:
            continue
        if decoded[0] == "EVENT" and len(decoded) >= 3:
            event = decoded[2]
            events.append(event)
            if len(events) >= args.count:
                if args.format == "jsonl":
                    for captured in events:
                        print(json.dumps(captured))
                elif args.format == "array":
                    print(json.dumps(events))
                else:
                    print(json.dumps(events[0]))
                return 0
        if decoded[0] == "EOSE":
            # No matching event yet; keep listening for new ones.
            continue
    print(f"timed out waiting for {args.count} event(s); captured {len(events)}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
