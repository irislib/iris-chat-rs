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


DEFAULT_IOS_SIMULATORS = ("Iris Chat iPhone", "Iris Chat iPhone 2")
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"
USER_VISIBLE_TIMEOUT_SECS = "60"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed four-device, two-user E2E mesh.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching local URLs.")
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
    parser.add_argument("--phone-serial", help="Preferred connected Android phone serial.")
    parser.add_argument("--serial", help="Alias for --phone-serial.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First values are used.")
    parser.add_argument(
        "--android-avd-only",
        action="store_true",
        help="Use Android AVDs only; ignore connected phones and Android serial environment variables.",
    )
    parser.add_argument("--android-avd", help="Preferred Android AVD for the second Android device.")
    parser.add_argument("--avd", help="Alias for --android-avd.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First values are used.")
    parser.add_argument(
        "--simulators",
        help=f"Two iOS simulator names, comma separated. Default: {', '.join(DEFAULT_IOS_SIMULATORS)}.",
    )
    parser.add_argument("--simulator-a", default=DEFAULT_IOS_SIMULATORS[0], help="Alice primary iOS simulator.")
    parser.add_argument("--simulator-b", default=DEFAULT_IOS_SIMULATORS[1], help="Bob linked iOS simulator.")
    parser.add_argument("--udids", help="Two iOS UDIDs, comma/space separated. Overrides simulator names.")
    parser.add_argument("--udid-a", help="Alice primary iOS UDID.")
    parser.add_argument("--udid-b", help="Bob linked iOS UDID.")
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


def short_stamp() -> str:
    return time.strftime("%H%M%S")


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


def discover_avds() -> list[str]:
    completed = run([str(ROOT_DIR / "scripts" / "run_android_emulators.sh"), "--list"])
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


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


def unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value and value not in seen:
            seen.add(value)
            result.append(value)
    return result


def select_android_targets(args: argparse.Namespace) -> tuple[dict[str, str], dict[str, str]]:
    serials: list[str] = []
    if not args.android_avd_only:
        explicit_serials = []
        if args.phone_serial:
            explicit_serials.append(args.phone_serial)
        if args.serial:
            explicit_serials.append(args.serial)
        explicit_serials.extend(split_list(args.serials))
        explicit_serials.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_SERIALS")))
        serials = unique(explicit_serials + connected_android_serials())

    explicit_avds = []
    if args.android_avd:
        explicit_avds.append(args.android_avd)
    if args.avd:
        explicit_avds.append(args.avd)
    explicit_avds.extend(split_list(args.avds))
    explicit_avds.extend(split_list(os.environ.get("IRIS_ANDROID_E2E_AVDS")))
    avds = unique(explicit_avds + discover_avds())

    targets: list[dict[str, str]] = []
    if serials:
        targets.append({"serial": serials[0]})
    if len(serials) >= 2:
        targets.append({"serial": serials[1]})
    for avd in avds:
        if len(targets) >= 2:
            break
        targets.append({"avd": avd})
    if len(targets) < 2:
        raise SystemExit(
            "Need two Android targets for mixed F17. Provide two AVDs, connect a phone and provide an AVD, "
            "or set --serials/--avds with two usable targets."
        )
    return targets[0], targets[1]


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
        raise SystemExit("Need two iOS simulator names or UDIDs for mixed F17.")
    return {"simulator": simulators[0]}, {"simulator": simulators[1]}


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    android_a, android_b = select_android_targets(args)
    ios_a, ios_b = select_ios_entries(args)
    if args.relay_mode == "public":
        port = args.relay_port or 0
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        reverse_relay = False
        relay_config = {
            "start": False,
            "port": port,
            "url": ios_url,
            "android_url": android_url,
            "set_id": f"mixed-multi-device-public-{artifact_dir.name}",
        }
    else:
        port = args.relay_port or free_tcp_port()
        ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
        default_android_url = (
            f"ws://127.0.0.1:{port}"
            if any("serial" in target for target in (android_a, android_b))
            else f"ws://10.0.2.2:{port}"
        )
        android_url = args.android_relay_url or args.relay_url or default_android_url
        reverse_relay = relay_uses_localhost(android_url)
        relay_config = {
            "start": True,
            "port": port,
            "label": f"iris.mixed-multi-device.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-multi-device-{artifact_dir.name}",
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
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **ios_a,
        },
        {
            "id": "bob1",
            "platform": "android",
            "run_id": "bob1",
            "user": "bob",
            "display_name": "Bob",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **android_b,
        },
        {
            "id": "alice2",
            "platform": "android",
            "run_id": "alice2",
            "user": "alice",
            "display_name": "Alice phone",
            "linked_to": "alice",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **android_a,
        },
        {
            "id": "bob2",
            "platform": "ios",
            "run_id": "bob2",
            "user": "bob",
            "display_name": "Bob phone",
            "linked_to": "bob",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **ios_b,
        },
    ]

    config = {
        "name": f"mixed-multi-device-mesh-{artifact_dir.name}",
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
    config_path = artifact_dir / "mixed-multi-device-mesh-config.json"
    write_json(config_path, config)
    return config_path


def harness(scenario: Scenario, device_id: str, action: str, **args: str) -> dict[str, str]:
    return scenario.harness(device_id, action, args=args)


def owner_npub(scenario: Scenario, user_id: str) -> str:
    return scenario.state["users"][user_id]["npub"]


def devices_for_user(scenario: Scenario, user_id: str) -> list[str]:
    return [
        device_id
        for device_id, device in scenario.state["devices"].items()
        if device.get("user") == user_id
    ]


def peer_send(
    scenario: Scenario,
    sender: str,
    recipient_user: str,
    message: str,
    *,
    wait_for_delivery: bool = True,
) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        peer_input=owner_npub(scenario, recipient_user),
        message=message,
        wait_for_delivery=str(wait_for_delivery).lower(),
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )


def peer_wait(
    scenario: Scenario,
    receiver: str,
    peer_user: str,
    message: str,
    *,
    direction: str,
    expected_count: int = 1,
) -> str:
    return harness(
        scenario,
        receiver,
        "wait_for_message_from_args",
        peer_input=owner_npub(scenario, peer_user),
        message=message,
        direction=direction,
        expected_count=str(expected_count),
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    ).get("matching_count", "")


def chat_send(scenario: Scenario, sender: str, chat_id: str, message: str) -> dict[str, str]:
    return harness(
        scenario,
        sender,
        "send_message_from_args",
        chat_id=chat_id,
        message=message,
        wait_for_delivery="true",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )


def chat_wait(
    scenario: Scenario,
    receiver: str,
    chat_id: str,
    message: str,
    *,
    direction: str,
    expected_count: int = 1,
) -> str:
    return harness(
        scenario,
        receiver,
        "wait_for_message_from_args",
        chat_id=chat_id,
        message=message,
        direction=direction,
        expected_count=str(expected_count),
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    ).get("matching_count", "")


def stop_app(scenario: Scenario, device_id: str) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
        return
    adb = str(scenario.adb())
    package_name = device.get("app_package") or ANDROID_APP_PACKAGE
    run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)


def restart_app(scenario: Scenario, device_id: str, *, wait_for_drain: bool = True) -> None:
    device = scenario.state["devices"][device_id]
    if device["platform"] == "ios":
        run(["xcrun", "simctl", "terminate", device["udid"], "to.iris.chat"], check=False)
        time.sleep(1)
        run(["xcrun", "simctl", "launch", device["udid"], "to.iris.chat"], check=False)
    else:
        adb = str(scenario.adb())
        package_name = device.get("app_package") or ANDROID_APP_PACKAGE
        run([adb, "-s", device["serial"], "shell", "am", "force-stop", package_name], env=scenario.scenario_env(), check=False)
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
            env=scenario.scenario_env(),
            check=False,
        )
    if wait_for_drain:
        drain_after_restart(scenario, device_id)


def drain_after_restart(scenario: Scenario, device_id: str) -> dict[str, str]:
    return harness(
        scenario,
        device_id,
        "report_logged_in_identity",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        relay_drain_runtime_only="true",
    )


def assert_linked_owner_matches(scenario: Scenario) -> dict[str, bool]:
    checks = {
        "alice": scenario.state["devices"]["alice1"]["owner_hex"] == scenario.state["devices"]["alice2"]["owner_hex"],
        "bob": scenario.state["devices"]["bob1"]["owner_hex"] == scenario.state["devices"]["bob2"]["owner_hex"],
    }
    failed = [user for user, ok in checks.items() if not ok]
    if failed:
        raise SystemExit(f"Linked owner mismatch for: {', '.join(failed)}")
    return checks


def create_shared_group(scenario: Scenario, flow_stamp: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=f"Mixed Multi Device {flow_stamp}",
        member_inputs=owner_npub(scenario, "bob"),
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    group = {
        "name": f"Mixed Multi Device {flow_stamp}",
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob-mesh"] = group
    scenario.save_state()
    for device_id in ("alice2", "bob1", "bob2"):
        harness(
            scenario,
            device_id,
            "wait_for_group_chat_from_args",
            chat_id=group["chat_id"],
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )
    for device_id in ("alice1", "alice2", "bob1", "bob2"):
        harness(
            scenario,
            device_id,
            "wait_for_group_member_count_from_args",
            chat_id=group["chat_id"],
            member_count="2",
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )
    return group


def direct_mesh_send(scenario: Scenario, sender: str, recipient_user: str, message: str) -> dict[str, str]:
    sender_user = scenario.state["devices"][sender]["user"]
    peer_send(scenario, sender, recipient_user, message)
    counts: dict[str, str] = {}
    for device_id in devices_for_user(scenario, sender_user):
        counts[device_id] = peer_wait(
            scenario,
            device_id,
            recipient_user,
            message,
            direction="outgoing",
        )
    for device_id in devices_for_user(scenario, recipient_user):
        counts[device_id] = peer_wait(
            scenario,
            device_id,
            sender_user,
            message,
            direction="incoming",
        )
    return counts


def group_mesh_send(scenario: Scenario, sender: str, chat_id: str, message: str) -> dict[str, str]:
    sender_user = scenario.state["devices"][sender]["user"]
    chat_send(scenario, sender, chat_id, message)
    counts: dict[str, str] = {}
    for device_id, device in scenario.state["devices"].items():
        direction = "outgoing" if device["user"] == sender_user else "incoming"
        counts[device_id] = chat_wait(
            scenario,
            device_id,
            chat_id,
            message,
            direction=direction,
        )
    return counts


def run_direct_mesh_phase(scenario: Scenario, flow_stamp: str) -> tuple[dict[str, str], dict[str, dict[str, str]]]:
    messages = {
        "alice_primary_to_bob": f"mesh-direct-alice-primary-{flow_stamp}",
        "alice_linked_to_bob": f"mesh-direct-alice-linked-{flow_stamp}",
        "bob_linked_to_alice": f"mesh-direct-bob-linked-{flow_stamp}",
    }
    counts = {
        "alice_primary_to_bob": direct_mesh_send(scenario, "alice1", "bob", messages["alice_primary_to_bob"]),
        "alice_linked_to_bob": direct_mesh_send(scenario, "alice2", "bob", messages["alice_linked_to_bob"]),
        "bob_linked_to_alice": direct_mesh_send(scenario, "bob2", "alice", messages["bob_linked_to_alice"]),
    }
    return messages, counts


def run_group_mesh_phase(scenario: Scenario, chat_id: str, flow_stamp: str) -> tuple[dict[str, str], dict[str, dict[str, str]]]:
    messages = {
        "alice_primary": f"mesh-group-alice-primary-{flow_stamp}",
        "alice_linked": f"mesh-group-alice-linked-{flow_stamp}",
        "bob_primary": f"mesh-group-bob-primary-{flow_stamp}",
        "bob_linked": f"mesh-group-bob-linked-{flow_stamp}",
    }
    counts = {
        "alice_primary": group_mesh_send(scenario, "alice1", chat_id, messages["alice_primary"]),
        "alice_linked": group_mesh_send(scenario, "alice2", chat_id, messages["alice_linked"]),
        "bob_primary": group_mesh_send(scenario, "bob1", chat_id, messages["bob_primary"]),
        "bob_linked": group_mesh_send(scenario, "bob2", chat_id, messages["bob_linked"]),
    }
    return messages, counts


def run_linked_offline_phase(scenario: Scenario, chat_id: str, flow_stamp: str) -> tuple[dict[str, str], dict[str, str]]:
    messages = {
        "alice_linked_offline_direct": f"mesh-offline-alice-linked-direct-{flow_stamp}",
        "alice_linked_offline_group": f"mesh-offline-alice-linked-group-{flow_stamp}",
        "bob_linked_offline_direct": f"mesh-offline-bob-linked-direct-{flow_stamp}",
        "bob_linked_offline_group": f"mesh-offline-bob-linked-group-{flow_stamp}",
    }
    counts: dict[str, str] = {}

    stop_app(scenario, "alice2")
    peer_send(scenario, "bob1", "alice", messages["alice_linked_offline_direct"])
    counts["alice1_direct_while_alice2_closed"] = peer_wait(
        scenario,
        "alice1",
        "bob",
        messages["alice_linked_offline_direct"],
        direction="incoming",
    )
    counts["bob2_self_sync_while_alice2_closed"] = peer_wait(
        scenario,
        "bob2",
        "alice",
        messages["alice_linked_offline_direct"],
        direction="outgoing",
    )
    chat_send(scenario, "bob2", chat_id, messages["alice_linked_offline_group"])
    counts["alice1_group_while_alice2_closed"] = chat_wait(
        scenario,
        "alice1",
        chat_id,
        messages["alice_linked_offline_group"],
        direction="incoming",
    )
    counts["bob1_group_self_sync_while_alice2_closed"] = chat_wait(
        scenario,
        "bob1",
        chat_id,
        messages["alice_linked_offline_group"],
        direction="outgoing",
    )
    restart_app(scenario, "alice2")
    counts["alice2_direct_after_restart"] = peer_wait(
        scenario,
        "alice2",
        "bob",
        messages["alice_linked_offline_direct"],
        direction="incoming",
    )
    counts["alice2_group_after_restart"] = chat_wait(
        scenario,
        "alice2",
        chat_id,
        messages["alice_linked_offline_group"],
        direction="incoming",
    )

    stop_app(scenario, "bob2")
    peer_send(scenario, "alice2", "bob", messages["bob_linked_offline_direct"])
    counts["bob1_direct_while_bob2_closed"] = peer_wait(
        scenario,
        "bob1",
        "alice",
        messages["bob_linked_offline_direct"],
        direction="incoming",
    )
    counts["alice1_self_sync_while_bob2_closed"] = peer_wait(
        scenario,
        "alice1",
        "bob",
        messages["bob_linked_offline_direct"],
        direction="outgoing",
    )
    chat_send(scenario, "alice1", chat_id, messages["bob_linked_offline_group"])
    counts["bob1_group_while_bob2_closed"] = chat_wait(
        scenario,
        "bob1",
        chat_id,
        messages["bob_linked_offline_group"],
        direction="incoming",
    )
    counts["alice2_group_self_sync_while_bob2_closed"] = chat_wait(
        scenario,
        "alice2",
        chat_id,
        messages["bob_linked_offline_group"],
        direction="outgoing",
    )
    restart_app(scenario, "bob2")
    counts["bob2_direct_after_restart"] = peer_wait(
        scenario,
        "bob2",
        "alice",
        messages["bob_linked_offline_direct"],
        direction="incoming",
    )
    counts["bob2_group_after_restart"] = chat_wait(
        scenario,
        "bob2",
        chat_id,
        messages["bob_linked_offline_group"],
        direction="incoming",
    )
    return messages, counts


def run_final_restart_phase(scenario: Scenario, chat_id: str, message: str) -> dict[str, str]:
    for device_id in ("alice1", "alice2", "bob1", "bob2"):
        restart_app(scenario, device_id)
    counts: dict[str, str] = {}
    for device_id, device in scenario.state["devices"].items():
        direction = "outgoing" if device["user"] == "alice" else "incoming"
        counts[device_id] = chat_wait(
            scenario,
            device_id,
            chat_id,
            message,
            direction=direction,
        )
    return counts


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    owner_checks = assert_linked_owner_matches(scenario)
    group = create_shared_group(scenario, flow_stamp)
    chat_id = group["chat_id"]
    direct_messages, direct_counts = run_direct_mesh_phase(scenario, flow_stamp)
    group_messages, group_counts = run_group_mesh_phase(scenario, chat_id, flow_stamp)
    offline_messages, offline_counts = run_linked_offline_phase(scenario, chat_id, flow_stamp)
    final_restart_counts = run_final_restart_phase(
        scenario,
        chat_id,
        offline_messages["bob_linked_offline_group"],
    )

    for device_id in ("alice1", "alice2", "bob1", "bob2"):
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
        "owner_checks": owner_checks,
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "direct_messages": direct_messages,
        "direct_counts": direct_counts,
        "group_messages": group_messages,
        "group_counts": group_counts,
        "linked_offline_messages": offline_messages,
        "linked_offline_counts": offline_counts,
        "final_restart_counts": final_restart_counts,
        "relay_mode": "local" if scenario.uses_local_relay() else "public",
        "relay_urls": scenario.relay_url(),
        "android_relay_urls": scenario.android_relay_url(),
        "devices": {
            device_id: {
                "platform": device["platform"],
                "user": device["user"],
                "serial": device.get("serial", ""),
                "udid": device.get("udid", ""),
                "owner_hex": device.get("owner_hex", ""),
                "device_hex": device.get("device_hex", ""),
            }
            for device_id, device in scenario.state["devices"].items()
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "mixed-multi-device-mesh-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    suffix = "public" if args.relay_mode == "public" else "local"
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-multi-device-mesh-{run_id}-{suffix}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        try:
            scenario.setup()
            result = run_flow(scenario, artifact_dir)
            print(json.dumps(result, indent=2, sort_keys=True))
        except BaseException as exc:
            failure = {
                "status": "failed",
                "artifact_dir": str(artifact_dir),
                "error": str(exc),
                "error_type": type(exc).__name__,
                "state": str(scenario.state_path),
            }
            write_json(artifact_dir / "mixed-multi-device-mesh-summary.json", failure)
            raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
