#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import time
from pathlib import Path
from typing import Any

from mobile_scenario import ROOT_DIR, Scenario


DEFAULT_SIMULATORS = ["Iris Chat iPhone", "Iris Chat iPhone 2"]
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"
USER_VISIBLE_TIMEOUT_SECS = os.environ.get("IRIS_E2E_USER_VISIBLE_TIMEOUT_SECS", "60")
USER_VISIBLE_TIMEOUT_MS = str(int(USER_VISIBLE_TIMEOUT_SECS) * 1000)


def run(command: list[str], *, cwd: Path = ROOT_DIR, check: bool = True) -> subprocess.CompletedProcess[str]:
    print("+ " + " ".join(command), flush=True)
    completed = subprocess.run(
        command,
        cwd=str(cwd),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.stdout:
        print(completed.stdout, end="")
    if check and completed.returncode != 0:
        raise SystemExit(completed.returncode)
    return completed


def split_name_list(value: str | None) -> list[str]:
    if not value:
        return []
    separator = "|" if "|" in value else ","
    return [part.strip() for part in value.split(separator) if part.strip()]


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def stamp() -> str:
    return time.strftime("%Y%m%dT%H%M%S")


def short_stamp() -> str:
    return time.strftime("%H%M%S")


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Restore an existing iOS profile with a secret key, then message.")
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed iOS harness artifacts. Requires matching local relay URL.")
    parser.add_argument("--keep-devices-open", action="store_true")
    parser.add_argument("--relay-mode", choices=("local", "public"), default="local")
    parser.add_argument(
        "--public-relays",
        default=os.environ.get("IRIS_E2E_RELAYS", DEFAULT_PUBLIC_RELAYS),
        help=f"Comma-separated public message servers. Default: {DEFAULT_PUBLIC_RELAYS}.",
    )
    parser.add_argument("--relay-port", type=int)
    parser.add_argument("--relay-url")
    parser.add_argument("--simulator", action="append", default=[], help="Simulator name. Pass twice for Alice and Bob.")
    parser.add_argument("--simulators", help="Two simulator names separated by comma or |.")
    parser.add_argument("--udid", action="append", default=[], help="Simulator/device UDID. Pass twice for Alice and Bob.")
    parser.add_argument("--udids", help="Two UDIDs separated by comma or |.")
    return parser.parse_args()


def validate_args(args: argparse.Namespace) -> None:
    if args.skip_build and args.relay_mode == "local" and not (args.relay_port or args.relay_url):
        raise SystemExit(
            "--skip-build with a local relay needs --relay-port or --relay-url. "
            "Restoring clears app data, so the debug app falls back to the relay baked into the installed app."
        )


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    if args.relay_mode == "public":
        port = args.relay_port or 0
        relay_url = args.relay_url or args.public_relays
        relay_config = {
            "start": False,
            "port": port,
            "url": relay_url,
            "set_id": f"ios-restore-profile-public-{artifact_dir.name}",
        }
    else:
        port = args.relay_port or free_tcp_port()
        relay_url = args.relay_url or f"ws://127.0.0.1:{port}"
        relay_config = {
            "port": port,
            "label": f"iris.ios-restore-profile.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"ios-restore-profile-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "url": relay_url,
        }
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
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
        }
        if use_udids:
            device["udid"] = udids[index]
        else:
            device["simulator"] = simulators[index]
        devices.append(device)

    config = {
        "name": f"ios-restore-profile-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay_config,
        "ios": {
            "build": not args.skip_build,
        },
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "ios-restore-profile-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, *, reset: bool = False, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, reset=reset, args=args)


def restart_ios_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
    time.sleep(1)
    run(["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"], check=False)


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    marker = short_stamp()
    alice = scenario.state["devices"]["alice1"]
    bob = scenario.state["devices"]["bob1"]
    restored_name = f"Existing Alice {marker}"
    alice_to_bob = f"ios-restore-existing-a2b-{marker}"
    bob_to_alice = f"ios-restore-existing-b2a-{marker}"

    harness(
        scenario,
        "alice1",
        "update_profile_metadata_from_args",
        display_name=restored_name,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    exported = harness(scenario, "alice1", "export_secret_key")
    secret_key = exported["secret_key"]

    harness(scenario, "bob1", "create_chat_from_args", peer_input=alice["owner_npub"])
    harness(
        scenario,
        "bob1",
        "wait_for_peer_profile_name_from_args",
        peer_pubkey_hex=alice["owner_hex"],
        display_name=restored_name,
        timeout_ms=USER_VISIBLE_TIMEOUT_MS,
    )

    restore = harness(
        scenario,
        "alice1",
        "restore_session_from_args",
        reset=True,
        secret_key=secret_key,
        expected_public_key_hex=alice["owner_hex"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    scenario.record_identity("alice1", restore)
    harness(
        scenario,
        "alice1",
        "wait_for_account_display_name_from_args",
        display_name=restored_name,
    )

    restart_ios_app(scenario, "alice1")
    harness(scenario, "alice1", "report_logged_in_identity")

    harness(
        scenario,
        "alice1",
        "send_message_from_args",
        peer_input=bob["owner_npub"],
        message=alice_to_bob,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    bob_receive = harness(
        scenario,
        "bob1",
        "wait_for_message_from_args",
        peer_input=alice["owner_npub"],
        message=alice_to_bob,
        direction="incoming",
        expected_count="1",
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )

    harness(
        scenario,
        "bob1",
        "send_message_from_args",
        peer_input=alice["owner_npub"],
        message=bob_to_alice,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    alice_receive = harness(
        scenario,
        "alice1",
        "wait_for_message_from_args",
        peer_input=bob["owner_npub"],
        message=bob_to_alice,
        direction="incoming",
        expected_count="1",
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )

    for device_id in ("alice1", "bob1"):
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
        "restored_public_key_hex": restore.get("public_key_hex", ""),
        "expected_public_key_hex": alice["owner_hex"],
        "restored_display_name": restored_name,
        "bob_profile_observed": True,
        "post_restore_messages": {
            "alice_to_bob": alice_to_bob,
            "bob_to_alice": bob_to_alice,
        },
        "matching_counts": {
            "bob_received_alice": bob_receive.get("matching_count", ""),
            "alice_received_bob": alice_receive.get("matching_count", ""),
        },
        "relay_mode": "local" if scenario.uses_local_relay() else "public",
        "relay_urls": scenario.relay_url(),
        "setup": "ios",
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "ios-restore-profile-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    validate_args(args)
    run_id = stamp()
    suffix = f"{args.relay_mode}-{run_id}"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-ios-restore-profile-{suffix}")).resolve()
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
