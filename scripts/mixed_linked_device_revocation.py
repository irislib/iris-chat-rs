#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
from typing import Any

from mixed_offline_restart_recovery import (
    connected_android_serials,
    free_tcp_port,
    harness,
    relay_uses_localhost,
    send_peer,
    short_stamp,
    split_list,
    stamp,
    wait_chat,
    wait_peer,
    write_json,
)
from mobile_scenario import ROOT_DIR, Scenario, run


DEFAULT_SIMULATORS = ("Iris Chat iPhone", "Iris Chat iPhone 2")
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android linked-device revocation E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
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
    parser.add_argument("--relay-port", type=int, help="Local message-server TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android linked device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument(
        "--android-avd-only",
        action="store_true",
        help="Use an Android AVD only; ignore connected phones and Android serial environment variables.",
    )
    parser.add_argument("--avd", help="Android AVD name for the linked device.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulators", help="Two iOS simulator names, comma separated.")
    parser.add_argument("--simulator-a", default=DEFAULT_SIMULATORS[0], help="Alice primary iOS simulator.")
    parser.add_argument("--simulator-b", default=DEFAULT_SIMULATORS[1], help="Bob iOS simulator.")
    parser.add_argument("--udids", help="Two iOS UDIDs, comma/space separated. Overrides simulator names.")
    parser.add_argument("--udid-a", help="Alice primary iOS UDID.")
    parser.add_argument("--udid-b", help="Bob iOS UDID.")
    return parser.parse_args()


def split_name_list(value: str | None) -> list[str]:
    if not value:
        return []
    return [part.strip() for part in re.split(r"[,|]+", value) if part.strip()]


def unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value and value not in seen:
            seen.add(value)
            result.append(value)
    return result


def discover_avds(limit: int) -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    avds = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if len(avds) < limit:
        raise SystemExit(
            f"Need {limit} Android AVD or connected Android device for mixed F10; found {len(avds)} AVDs. "
            "Set --serial, --avd, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return avds[:limit]


def select_android_target(args: argparse.Namespace) -> dict[str, str]:
    serials = []
    if not args.android_avd_only:
        if args.serial:
            serials.append(args.serial)
        serials.extend(split_list(args.serials))
        serials.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS")))
        serials = unique(serials + connected_android_serials())
    if serials:
        return {"serial": serials[0]}

    avds = []
    if args.avd:
        avds.append(args.avd)
    avds.extend(split_list(args.avds))
    avds.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS")))
    if avds:
        return {"avd": unique(avds)[0]}

    return {"avd": discover_avds(1)[0]}


def select_ios_entries(args: argparse.Namespace) -> tuple[dict[str, str], dict[str, str]]:
    udids = split_list(args.udids)
    if args.udid_a:
        udids.insert(0, args.udid_a)
    if args.udid_b:
        udids.insert(1 if udids else 0, args.udid_b)
    if len(udids) >= 2:
        return {"udid": udids[0]}, {"udid": udids[1]}

    simulators = split_name_list(args.simulators)
    if len(simulators) < 2:
        simulators = [args.simulator_a, args.simulator_b]
    if len(simulators) < 2:
        raise SystemExit("Need two iOS simulator names or UDIDs for mixed F10.")
    return {"simulator": simulators[0]}, {"simulator": simulators[1]}


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    android_target = select_android_target(args)
    alice_ios, bob_ios = select_ios_entries(args)

    if args.relay_mode == "public":
        port = args.relay_port or 0
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        reverse_relay = False
        relay_config = {
            "start": False,
            "port": port,
            "set_id": f"mixed-linked-revocation-public-{artifact_dir.name}",
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
            "label": f"iris.mixed-linked-revocation.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-linked-revocation-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        }

    devices: list[dict[str, Any]] = [
        {
            "id": "alice1",
            "platform": "ios",
            "run_id": "alice1",
            "user": "alice",
            "display_name": "Alice",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            **alice_ios,
        },
        {
            "id": "bob1",
            "platform": "ios",
            "run_id": "bob1",
            "user": "bob",
            "display_name": "Bob",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            **bob_ios,
        },
        {
            "id": "alice2",
            "platform": "android",
            "run_id": "alice2",
            "user": "alice",
            "linked_to": "alice",
            "display_name": "Alice Phone",
            "reset": True,
            "relay_drain_timeout_secs": 240,
            "authorization_timeout_secs": 300,
            **android_target,
        },
    ]

    config = {
        "name": f"mixed-linked-revocation-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": relay_config,
        "ios": {"build": not args.skip_build},
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
    config_path = artifact_dir / "mixed-linked-revocation-config.json"
    write_json(config_path, config)
    return config_path


def wait_direct_by_chat_id(
    scenario: Scenario,
    device_id: str,
    peer_owner_hex: str,
    message: str,
    *,
    direction: str,
) -> dict[str, str]:
    return wait_chat(
        scenario,
        device_id,
        peer_owner_hex,
        message,
        direction=direction,
        expected_count=1,
    )


def assert_direct_absent(
    scenario: Scenario,
    device_id: str,
    peer_owner_hex: str,
    message: str,
    *,
    timeout_ms: int = 30_000,
) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "assert_message_absent_from_args",
        chat_id=peer_owner_hex,
        message=message,
        direction="any",
        timeout_ms=str(timeout_ms),
    )


def expect_send_rejected_to_bob(scenario: Scenario, message: str) -> dict[str, str]:
    return harness(
        scenario,
        "alice2",
        "expect_send_rejected_from_args",
        peer_input=scenario.state["devices"]["bob1"]["owner_npub"],
        message=message,
    )


def report_device_roster(scenario: Scenario, device_id: str) -> dict[str, str]:
    return harness(scenario, device_id, "report_device_roster_snapshot")


def roster_device_hexes(roster: dict[str, str]) -> set[str]:
    devices = roster.get("devices", "")
    result: set[str] = set()
    for row in devices.split("|"):
        fields = [field.strip() for field in row.split(",")]
        if fields and fields[0]:
            result.add(fields[0].lower())
    return result


def require_roster_contains(roster: dict[str, str], device_hex: str, label: str) -> None:
    if device_hex.lower() not in roster_device_hexes(roster):
        raise AssertionError(
            f"{label}: expected roster to contain device {device_hex}; got {roster.get('devices', '')}"
        )


def require_roster_omits(roster: dict[str, str], device_hex: str, label: str) -> None:
    if device_hex.lower() in roster_device_hexes(roster):
        raise AssertionError(
            f"{label}: expected roster to omit device {device_hex}; got {roster.get('devices', '')}"
        )


def revoke_linked_device(scenario: Scenario) -> dict[str, str]:
    return harness(
        scenario,
        "alice1",
        "remove_authorized_device_from_args",
        device_input=scenario.state["devices"]["alice2"]["device_hex"],
    )


def wait_revoked(scenario: Scenario) -> dict[str, str]:
    return harness(
        scenario,
        "alice2",
        "wait_for_revoked_state",
    )


def run_flow(scenario: Scenario, artifact_dir: Path, relay_mode: str) -> dict[str, Any]:
    flow_stamp = short_stamp()
    alice = scenario.state["devices"]["alice1"]
    alice_linked = scenario.state["devices"]["alice2"]
    bob = scenario.state["devices"]["bob1"]
    bob_owner_hex = bob["owner_hex"]
    alice_owner_hex = alice["owner_hex"]

    primary_message = f"mixed-revoke-primary-before-{flow_stamp}"
    send_peer(scenario, "alice1", "bob1", primary_message)
    primary_counts = {
        "bob1": wait_peer(scenario, "bob1", "alice1", primary_message, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice2": wait_direct_by_chat_id(
            scenario,
            "alice2",
            bob_owner_hex,
            primary_message,
            direction="outgoing",
        ).get("matching_count", ""),
    }

    bob_reply = f"mixed-revoke-bob-before-{flow_stamp}"
    send_peer(scenario, "bob1", "alice1", bob_reply)
    reply_counts = {
        "alice1": wait_peer(scenario, "alice1", "bob1", bob_reply, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice2": wait_direct_by_chat_id(
            scenario,
            "alice2",
            bob_owner_hex,
            bob_reply,
            direction="incoming",
        ).get("matching_count", ""),
    }

    linked_message = f"mixed-revoke-linked-before-{flow_stamp}"
    send_peer(scenario, "alice2", "bob1", linked_message)
    linked_counts = {
        "bob1": wait_peer(scenario, "bob1", "alice1", linked_message, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice1": wait_direct_by_chat_id(
            scenario,
            "alice1",
            bob_owner_hex,
            linked_message,
            direction="outgoing",
        ).get("matching_count", ""),
    }

    owner_roster_before_revoke = report_device_roster(scenario, "alice1")
    require_roster_contains(
        owner_roster_before_revoke,
        alice_linked["device_hex"],
        "owner roster before revoke",
    )
    revoke = revoke_linked_device(scenario)
    if revoke.get("device_removed") != "true":
        raise AssertionError(f"owner revoke did not remove linked device from roster: {revoke}")
    owner_roster_after_revoke = report_device_roster(scenario, "alice1")
    require_roster_omits(
        owner_roster_after_revoke,
        alice_linked["device_hex"],
        "owner roster after revoke",
    )
    revoked = wait_revoked(scenario)

    rejected_message = f"mixed-revoke-linked-after-{flow_stamp}"
    rejected = expect_send_rejected_to_bob(scenario, rejected_message)
    rejected_absent = {
        "bob1": assert_direct_absent(scenario, "bob1", alice_owner_hex, rejected_message),
        "alice1": assert_direct_absent(scenario, "alice1", bob_owner_hex, rejected_message),
    }

    final_message = f"mixed-revoke-primary-after-{flow_stamp}"
    send_peer(scenario, "alice1", "bob1", final_message)
    final_counts = {
        "bob1": wait_peer(scenario, "bob1", "alice1", final_message, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice1": wait_direct_by_chat_id(
            scenario,
            "alice1",
            bob_owner_hex,
            final_message,
            direction="outgoing",
        ).get("matching_count", ""),
    }
    revoked_absent_final = assert_direct_absent(scenario, "alice2", bob_owner_hex, final_message)

    for device_id in ("alice1", "alice2", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "relay_mode": relay_mode,
        "relay_urls": scenario.relay_url(),
        "android_relay_urls": scenario.android_relay_url(),
        "primary_message": primary_message,
        "primary_counts": primary_counts,
        "bob_reply": bob_reply,
        "reply_counts": reply_counts,
        "linked_message": linked_message,
        "linked_counts": linked_counts,
        "owner_roster_before_revoke": owner_roster_before_revoke,
        "revoke": revoke,
        "owner_roster_after_revoke": owner_roster_after_revoke,
        "revoked": revoked,
        "rejected_message": rejected_message,
        "rejected": rejected,
        "rejected_absent": rejected_absent,
        "final_message": final_message,
        "final_counts": final_counts,
        "revoked_absent_final": revoked_absent_final,
        "devices": {
            "alice1": {
                "platform": alice["platform"],
                "serial": alice.get("serial", ""),
                "udid": alice.get("udid", ""),
                "owner_hex": alice.get("owner_hex", ""),
                "device_hex": alice.get("device_hex", ""),
            },
            "alice2": {
                "platform": alice_linked["platform"],
                "serial": alice_linked.get("serial", ""),
                "udid": alice_linked.get("udid", ""),
                "owner_hex": alice_linked.get("owner_hex", ""),
                "device_hex": alice_linked.get("device_hex", ""),
            },
            "bob1": {
                "platform": bob["platform"],
                "serial": bob.get("serial", ""),
                "udid": bob.get("udid", ""),
                "owner_hex": bob.get("owner_hex", ""),
                "device_hex": bob.get("device_hex", ""),
            },
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-linked-revocation-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-linked-revocation-{run_id}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        scenario.setup()
        result = run_flow(scenario, artifact_dir, args.relay_mode)
        print(json.dumps(result, indent=2, sort_keys=True))
    except BaseException as error:
        failure = {
            "status": "failed",
            "artifact_dir": str(artifact_dir),
            "error": str(error),
        }
        write_json(artifact_dir / "mixed-linked-revocation-summary.json", failure)
        raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
