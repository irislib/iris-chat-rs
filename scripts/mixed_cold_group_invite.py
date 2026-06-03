#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import subprocess
import time
from pathlib import Path
from typing import Any

from mobile_scenario import ROOT_DIR, Scenario, discover_android_sdk_dir, run


DEFAULT_SIMULATOR = "Iris Chat iPhone"
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android cold group invite E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--alice-platform", choices=("ios", "android"), default="ios")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-mode", choices=("local", "public"), default="local")
    parser.add_argument(
        "--public-relays",
        default=os.environ.get("IRIS_E2E_RELAYS", DEFAULT_PUBLIC_RELAYS),
        help=f"Comma-separated public message servers. Default: {DEFAULT_PUBLIC_RELAYS}.",
    )
    parser.add_argument("--relay-port", type=int, help="Local relay TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="Relay URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="Relay URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument("--avd", help="Android AVD name.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulator", default=DEFAULT_SIMULATOR, help=f"iOS simulator name. Default: {DEFAULT_SIMULATOR}.")
    parser.add_argument("--udid", help="iOS simulator/device UDID.")
    return parser.parse_args()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


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


def relay_uses_localhost(relay_url: str) -> bool:
    return "://127.0.0.1:" in relay_url or "://localhost:" in relay_url


def discover_avds(limit: int) -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    avds = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if len(avds) < limit:
        raise SystemExit(
            f"Need {limit} Android AVD or connected Android device for mixed F16; found {len(avds)} AVDs. "
            "Set --serial, --avd, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return avds[:limit]


def connected_android_serials() -> list[str]:
    sdk_dir = discover_android_sdk_dir()
    if sdk_dir is None:
        return []
    adb = sdk_dir / "platform-tools" / "adb"
    if not adb.exists():
        return []
    completed = subprocess.run(
        [str(adb), "devices", "-l"],
        cwd=str(ROOT_DIR),
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        return []
    serials: list[str] = []
    for line in completed.stdout.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2 and parts[1] == "device" and not parts[0].startswith("emulator-"):
            serials.append(parts[0])
    return serials


def select_android_target(args: argparse.Namespace) -> dict[str, str]:
    serials = []
    if args.serial:
        serials.append(args.serial)
    serials.extend(split_list(args.serials))
    serials.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS")))
    if serials:
        return {"serial": serials[0]}

    avds = []
    if args.avd:
        avds.append(args.avd)
    avds.extend(split_list(args.avds))
    avds.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS")))
    if avds:
        return {"avd": avds[0]}

    connected = connected_android_serials()
    if connected:
        return {"serial": connected[0]}

    return {"avd": discover_avds(1)[0]}


def ios_device_entry(device_id: str, user: str, display_name: str, args: argparse.Namespace) -> dict[str, Any]:
    device: dict[str, Any] = {
        "id": device_id,
        "platform": "ios",
        "run_id": device_id,
        "user": user,
        "display_name": display_name,
        "reset": True,
        "relay_drain_timeout_secs": 240,
    }
    if args.udid:
        device["udid"] = args.udid
    else:
        device["simulator"] = args.simulator
    return device


def android_device_entry(
    device_id: str,
    user: str,
    display_name: str,
    target: dict[str, str],
) -> dict[str, Any]:
    device: dict[str, Any] = {
        "id": device_id,
        "platform": "android",
        "run_id": device_id,
        "user": user,
        "display_name": display_name,
        "reset": True,
        "relay_drain_timeout_secs": 240,
    }
    device.update(target)
    return device


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    android_target = select_android_target(args)
    if args.relay_mode == "public":
        port = args.relay_port or 0
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        reverse_relay = False
        relay_config = {
            "start": False,
            "port": port,
            "set_id": f"mixed-cold-group-public-{artifact_dir.name}",
            "android_url": android_url,
            "url": ios_url,
        }
    else:
        port = args.relay_port or free_tcp_port()
        ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
        default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
        android_url = args.android_relay_url or args.relay_url or default_android_url
        reverse_relay = relay_uses_localhost(android_url)
        relay_config = {
            "port": port,
            "label": f"iris.mixed-cold-group.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-cold-group-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        }

    ios_entry = ios_device_entry(
        "alice1" if args.alice_platform == "ios" else "bob1",
        "alice" if args.alice_platform == "ios" else "bob",
        "Alice" if args.alice_platform == "ios" else "Bob",
        args,
    )
    android_entry = android_device_entry(
        "alice1" if args.alice_platform == "android" else "bob1",
        "alice" if args.alice_platform == "android" else "bob",
        "Alice" if args.alice_platform == "android" else "Bob",
        android_target,
    )
    devices = [ios_entry, android_entry]

    config = {
        "name": f"mixed-cold-group-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay_config,
        "ios": {
            "build": not args.skip_build,
        },
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": reverse_relay,
        },
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "mixed-cold-group-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


def create_cold_group(scenario: Scenario, group_name: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )
    group_state = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob-cold"] = group_state
    scenario.save_state()
    harness(
        scenario,
        "bob1",
        "wait_for_group_chat_from_args",
        chat_id=group_state["chat_id"],
        timeout_secs="300",
    )
    return group_state


def send_chat(scenario: Scenario, sender: str, chat_id: str, message: str) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        chat_id=chat_id,
        message=message,
        wait_for_delivery="true",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs="240",
    )


def wait_chat(scenario: Scenario, receiver: str, chat_id: str, message: str) -> dict[str, str]:
    return harness(
        scenario,
        receiver,
        "wait_for_message_from_args",
        chat_id=chat_id,
        message=message,
        direction="incoming",
        expected_count="1",
        timeout_secs="240",
    )


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = time.strftime("%H%M%S")
    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    creator_platform = alice["platform"]
    recipient_platform = bob["platform"]
    group = create_cold_group(
        scenario,
        f"Mixed Cold Group {creator_platform}-to-{recipient_platform} {flow_stamp}",
    )
    chat_id = group["chat_id"]
    messages = {
        "alice_group": f"mixed-cold-group-alice-{creator_platform}-{flow_stamp}",
        "bob_group": f"mixed-cold-group-bob-{recipient_platform}-{flow_stamp}",
    }

    send_chat(scenario, "alice1", chat_id, messages["alice_group"])
    bob_count = wait_chat(scenario, "bob1", chat_id, messages["alice_group"]).get(
        "matching_count",
        "",
    )
    send_chat(scenario, "bob1", chat_id, messages["bob_group"])
    alice_count = wait_chat(scenario, "alice1", chat_id, messages["bob_group"]).get(
        "matching_count",
        "",
    )

    for device_id in ("alice1", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "creator_platform": creator_platform,
        "recipient_platform": recipient_platform,
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "messages": messages,
        "duplicate_counts": {
            "recipient_group_from_creator": bob_count,
            "creator_group_from_recipient": alice_count,
        },
        "relay_mode": scenario.relay_config().get("start", True) and "local" or "public",
        "relay_urls": scenario.relay_config().get("url", ""),
        "android_relay_urls": scenario.relay_config().get("android_url", ""),
        "devices": {
            "alice1": {
                "platform": alice["platform"],
                "serial": alice.get("serial", ""),
                "udid": alice.get("udid", ""),
            },
            "bob1": {
                "platform": bob["platform"],
                "serial": bob.get("serial", ""),
                "udid": bob.get("udid", ""),
            },
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-cold-group-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    mode_suffix = "public" if args.relay_mode == "public" else "local"
    suffix = f"{mode_suffix}-{run_id}-{args.alice_platform}-creator"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-cold-group-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir)
        print(json.dumps(result, indent=2, sort_keys=True))
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
