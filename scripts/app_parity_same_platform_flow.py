#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import time
from pathlib import Path
from typing import Any

from mixed_app_parity_flow import DEFAULT_PUBLIC_RELAYS, run_flow, write_json
from mobile_scenario import Scenario


DEFAULT_SIMULATORS = ["Iris Chat iPhone", "Iris Chat iPhone 2"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Same-platform app-level parity E2E.")
    parser.add_argument("--platform", choices=("ios", "android"), required=True)
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--relay-mode", choices=("local", "public"), default="local")
    parser.add_argument(
        "--public-relays",
        default=os.environ.get("IRIS_E2E_RELAYS", DEFAULT_PUBLIC_RELAYS),
        help=f"Comma-separated public message servers. Default: {DEFAULT_PUBLIC_RELAYS}.",
    )
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local relay TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="Relay URL compiled into the iOS harness or Android app.")
    parser.add_argument("--android-relay-url", help="Relay URL compiled into the Android debug app.")
    parser.add_argument("--serials", help="One or two adb serials, space/comma separated. Can be combined with --avds.")
    parser.add_argument("--avds", help="One or two Android AVD names, space/comma separated. Can be combined with --serials.")
    parser.add_argument("--simulator", action="append", default=[], help="Simulator name. Pass twice for Alice and Bob.")
    parser.add_argument("--simulators", help="Two simulator names separated by comma or |.")
    parser.add_argument("--udid", action="append", default=[], help="Simulator/device UDID. Pass twice for Alice and Bob.")
    parser.add_argument("--udids", help="Two UDIDs separated by comma or |.")
    return parser.parse_args()


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%S")


def split_list(value: str | None) -> list[str]:
    if not value:
        return []
    return [part for part in re.split(r"[\s,|]+", value.strip()) if part]


def split_name_list(value: str | None) -> list[str]:
    if not value:
        return []
    return [part.strip() for part in re.split(r"[,|]+", value) if part.strip()]


def relay_uses_localhost(relay_url: str) -> bool:
    return "://127.0.0.1:" in relay_url or "://localhost:" in relay_url


def relay_config(args: argparse.Namespace, artifact_dir: Path, *, has_android_serial: bool) -> tuple[dict[str, Any], str, str, bool]:
    if args.relay_mode == "public":
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        relay = {
            "start": False,
            "port": args.relay_port or 0,
            "url": ios_url,
            "android_url": android_url,
            "set_id": f"{args.platform}-app-parity-public-{artifact_dir.name}",
        }
        return relay, ios_url, android_url, False

    port = args.relay_port or free_tcp_port()
    ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
    default_android_url = f"ws://127.0.0.1:{port}" if has_android_serial else f"ws://10.0.2.2:{port}"
    android_url = args.android_relay_url or args.relay_url or default_android_url
    relay = {
        "start": True,
        "port": port,
        "label": f"iris.{args.platform}-app-parity.{artifact_dir.name}.relay",
        "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
        "log_file": str(artifact_dir / "scenario" / "relay.log"),
        "set_id": f"{args.platform}-app-parity-{artifact_dir.name}",
        "bind_host": "0.0.0.0",
        "url": ios_url if args.platform == "ios" else android_url,
        "android_url": android_url,
    }
    return relay, ios_url, android_url, relay_uses_localhost(android_url)


def ios_devices(args: argparse.Namespace) -> list[dict[str, Any]]:
    udids = args.udid + split_name_list(args.udids)
    simulators = args.simulator + split_name_list(args.simulators)
    if len(udids) < 2 and len(simulators) < 2:
        simulators = DEFAULT_SIMULATORS
    use_udids = len(udids) >= 2

    devices: list[dict[str, Any]] = []
    for index, (device_id, user, display_name) in enumerate(
        [
            ("alice1", "alice", "Alice"),
            ("bob1", "bob", "Bob"),
        ]
    ):
        device: dict[str, Any] = {
            "id": device_id,
            "platform": "ios",
            "run_id": device_id,
            "user": user,
            "display_name": display_name,
            "reset": True,
            "relay_drain_timeout_secs": 240,
        }
        if use_udids:
            device["udid"] = udids[index]
        else:
            device["simulator"] = simulators[index]
        devices.append(device)
    return devices


def android_targets(args: argparse.Namespace) -> list[dict[str, str]]:
    serials = split_list(args.serials) or split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS"))
    avds = split_list(args.avds) or split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS"))
    selected: list[dict[str, str]] = [{"serial": serial} for serial in serials[:2]]
    selected.extend({"avd": avd} for avd in avds[: 2 - len(selected)])
    if len(selected) < 2:
        raise SystemExit(
            "Need two Android targets. Set --serials, --avds, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return selected[:2]


def android_devices(args: argparse.Namespace) -> list[dict[str, Any]]:
    targets = android_targets(args)
    devices: list[dict[str, Any]] = []
    for index, (device_id, user, display_name) in enumerate(
        [
            ("alice1", "alice", "Alice"),
            ("bob1", "bob", "Bob"),
        ]
    ):
        device: dict[str, Any] = {
            "id": device_id,
            "platform": "android",
            "run_id": device_id,
            "user": user,
            "display_name": display_name,
            "reset": True,
            "relay_drain_timeout_secs": 240,
        }
        device.update(targets[index])
        devices.append(device)
    return devices


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    devices = ios_devices(args) if args.platform == "ios" else android_devices(args)
    has_android_serial = any(device.get("platform") == "android" and device.get("serial") for device in devices)
    relay, _ios_url, _android_url, reverse_relay = relay_config(
        args,
        artifact_dir,
        has_android_serial=has_android_serial,
    )
    config: dict[str, Any] = {
        "name": f"{args.platform}-app-parity-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay,
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    if args.platform == "ios":
        config["ios"] = {
            "build": not args.skip_build,
        }
    else:
        config["android"] = {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": reverse_relay,
        }
    config_path = artifact_dir / f"{args.platform}-app-parity-config.json"
    write_json(config_path, config)
    return config_path


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = f"{args.relay_mode}-{run_id}"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-{args.platform}-app-parity-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir)
        result["setup"] = args.platform
        write_json(artifact_dir / f"{args.platform}-app-parity-summary.json", result)
        print(json.dumps(result, indent=2, sort_keys=True))
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
