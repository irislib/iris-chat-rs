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

from mobile_scenario import ANDROID_APP_PACKAGE, ROOT_DIR, Scenario, discover_android_sdk_dir, run


DEFAULT_SIMULATOR = "Iris Chat iPhone"
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"
USER_VISIBLE_TIMEOUT_SECS = os.environ.get("IRIS_E2E_USER_VISIBLE_TIMEOUT_SECS", "60")
USER_VISIBLE_TIMEOUT_MS = str(int(USER_VISIBLE_TIMEOUT_SECS) * 1000)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android restore with a secret key, then message.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--source-platform", choices=("ios", "android"), default="ios")
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


def validate_args(args: argparse.Namespace) -> None:
    if (
        args.skip_build
        and args.relay_mode == "local"
        and not (args.relay_port or args.relay_url or args.android_relay_url)
    ):
        raise SystemExit(
            "--skip-build with a local relay needs --relay-port, --relay-url, or --android-relay-url. "
            "Restoring clears app data, so apps fall back to the relay baked into the installed artifacts."
        )


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%S")


def short_stamp() -> str:
    return time.strftime("%H%M%S")


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
            f"Need {limit} Android AVD or connected Android device for mixed F15; found {len(avds)} AVDs. "
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
        "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
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
        "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
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
            "set_id": f"mixed-restore-profile-public-{artifact_dir.name}",
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
            "label": f"iris.mixed-restore-profile.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-restore-profile-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        }

    source_id = "source1"
    restore_id = "restore1"
    ios_role_id = source_id if args.source_platform == "ios" else restore_id
    android_role_id = source_id if args.source_platform == "android" else restore_id
    ios_display = "Alice" if args.source_platform == "ios" else "Restore Target"
    android_display = "Alice" if args.source_platform == "android" else "Restore Target"

    devices = [
        ios_device_entry(ios_role_id, f"{ios_role_id}_user", ios_display, args),
        android_device_entry(android_role_id, f"{android_role_id}_user", android_display, android_target),
    ]

    config = {
        "name": f"mixed-restore-profile-{artifact_dir.name}",
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
    config_path = artifact_dir / "mixed-restore-profile-config.json"
    write_json(config_path, config)
    return config_path


def harness(
    scenario: Scenario,
    device_id: str,
    action: str,
    *,
    reset: bool = False,
    **args: str,
) -> dict[str, str]:
    return scenario.harness(device_id, action, reset=reset, args=args)


def restart_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
        time.sleep(1)
        run(["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"], check=False)
        return

    adb = str(scenario.adb())
    package_name = device.get("app_package") or ANDROID_APP_PACKAGE
    run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], check=False)
    time.sleep(1)
    run(
        [
            adb,
            "-s",
            device["serial"],
            "shell",
            "monkey",
            "-p",
            package_name,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ],
        check=False,
    )


def run_flow(scenario: Scenario, artifact_dir: Path, source_platform: str) -> dict[str, Any]:
    marker = short_stamp()
    source_id = "source1"
    restore_id = "restore1"
    restore_platform = "android" if source_platform == "ios" else "ios"
    restored_name = f"Existing Alice {marker}"
    alice_to_bob = f"mixed-restore-a2b-{source_platform}-to-{restore_platform}-{marker}"
    bob_to_alice = f"mixed-restore-b2a-{source_platform}-to-{restore_platform}-{marker}"

    harness(
        scenario,
        source_id,
        "update_profile_metadata_from_args",
        display_name=restored_name,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    exported = harness(scenario, source_id, "export_secret_key")
    secret_key = exported["secret_key"]
    alice_before_restore = dict(scenario.state["devices"][source_id])
    alice_owner_hex = alice_before_restore["owner_hex"]
    alice_owner_npub = alice_before_restore["owner_npub"]

    restore = harness(
        scenario,
        restore_id,
        "restore_session_from_args",
        reset=True,
        secret_key=secret_key,
        expected_public_key_hex=alice_owner_hex,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    scenario.record_identity(restore_id, restore)
    harness(
        scenario,
        restore_id,
        "wait_for_account_display_name_from_args",
        display_name=restored_name,
    )
    restart_app(scenario, restore_id)
    harness(scenario, restore_id, "report_logged_in_identity")

    bob = harness(
        scenario,
        source_id,
        "create_account_and_report_identity",
        reset=True,
        display_name="Bob",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    scenario.record_identity(source_id, bob)
    bob_after_reset = scenario.state["devices"][source_id]

    harness(scenario, source_id, "create_chat_from_args", peer_input=alice_owner_npub)
    harness(scenario, restore_id, "create_chat_from_args", peer_input=bob_after_reset["owner_npub"])
    harness(
        scenario,
        source_id,
        "wait_for_peer_profile_name_from_args",
        peer_pubkey_hex=alice_owner_hex,
        display_name=restored_name,
        timeout_ms=USER_VISIBLE_TIMEOUT_MS,
    )
    harness(scenario, source_id, "wait_for_peer_transport_ready_from_args", peer_input=alice_owner_npub)
    harness(scenario, restore_id, "wait_for_peer_transport_ready_from_args", peer_input=bob_after_reset["owner_npub"])

    harness(
        scenario,
        restore_id,
        "send_message_from_args",
        peer_input=bob_after_reset["owner_npub"],
        message=alice_to_bob,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    bob_receive = harness(
        scenario,
        source_id,
        "wait_for_message_from_args",
        peer_input=alice_owner_npub,
        message=alice_to_bob,
        direction="incoming",
        expected_count="1",
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )

    harness(
        scenario,
        source_id,
        "send_message_from_args",
        peer_input=alice_owner_npub,
        message=bob_to_alice,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    alice_receive = harness(
        scenario,
        restore_id,
        "wait_for_message_from_args",
        peer_input=bob_after_reset["owner_npub"],
        message=bob_to_alice,
        direction="incoming",
        expected_count="1",
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )

    for device_id in (restore_id, source_id):
        harness(
            scenario,
            device_id,
            "report_runtime_debug_snapshot",
            wait_for_relay_drain="true",
            wait_for_runtime_idle="true",
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
            runtime_idle_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )
        harness(
            scenario,
            device_id,
            "report_persisted_protocol_snapshot",
            wait_for_relay_drain="true",
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "source_platform": source_platform,
        "restore_platform": restore_platform,
        "restored_public_key_hex": restore.get("public_key_hex", ""),
        "expected_public_key_hex": alice_owner_hex,
        "restored_display_name": restored_name,
        "post_restore_messages": {
            "alice_to_bob": alice_to_bob,
            "bob_to_alice": bob_to_alice,
        },
        "matching_counts": {
            "bob_received_alice": bob_receive.get("matching_count", ""),
            "alice_received_bob": alice_receive.get("matching_count", ""),
        },
        "relay_mode": scenario.relay_config().get("start", True) and "local" or "public",
        "relay_urls": scenario.relay_config().get("url", ""),
        "android_relay_urls": scenario.relay_config().get("android_url", ""),
        "devices": {
            "restored_alice": {
                "platform": scenario.state["devices"][restore_id]["platform"],
                "serial": scenario.state["devices"][restore_id].get("serial", ""),
                "udid": scenario.state["devices"][restore_id].get("udid", ""),
            },
            "bob": {
                "platform": bob_after_reset["platform"],
                "serial": bob_after_reset.get("serial", ""),
                "udid": bob_after_reset.get("udid", ""),
            },
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-restore-profile-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    validate_args(args)
    run_id = stamp()
    mode_suffix = "public" if args.relay_mode == "public" else "local"
    suffix = f"{mode_suffix}-{run_id}-{args.source_platform}-source"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-restore-profile-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir, args.source_platform)
        print(json.dumps(result, indent=2, sort_keys=True))
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
