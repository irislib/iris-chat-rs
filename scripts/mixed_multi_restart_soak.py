#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import time
from pathlib import Path
from typing import Any

from mixed_offline_restart_recovery import (
    connected_android_serials,
    discover_avds,
    drain_after_restart,
    free_tcp_port,
    harness,
    relay_uses_localhost,
    restart_app,
    send_chat,
    send_peer,
    short_stamp,
    split_list,
    stamp,
    stop_app,
    wait_chat,
    wait_peer,
    write_json,
)
from mobile_scenario import ROOT_DIR, Scenario


DEFAULT_SIMULATORS = ("Iris Chat iPhone", "Iris Chat iPhone 2")
DEFAULT_PUBLIC_RELAYS = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://temp.iris.to"
USER_VISIBLE_TIMEOUT_SECS = "60"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android multi-restart soak E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--iterations", type=int, default=2, help="Number of restart/message cycles. Default: 2.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-mode", choices=("local", "public"), default="local", help="Use a local message server or the public relay set.")
    parser.add_argument("--public-relays", default=os.environ.get("IRIS_E2E_RELAYS", DEFAULT_PUBLIC_RELAYS), help="Comma-separated public message servers for --relay-mode public.")
    parser.add_argument("--relay-port", type=int, help="Local message-server TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
    parser.add_argument(
        "--android-avd-only",
        action="store_true",
        help="Use Android AVDs only; ignore connected phones and Android serial environment variables.",
    )
    parser.add_argument("--avd", help="Android AVD name.")
    parser.add_argument("--avds", help="Android AVD names, space/comma separated. First value is used.")
    parser.add_argument("--simulators", help="Two iOS simulator names, comma separated.")
    parser.add_argument("--simulator-a", default=DEFAULT_SIMULATORS[0], help="Bob iOS simulator.")
    parser.add_argument("--simulator-b", default=DEFAULT_SIMULATORS[1], help="Carol iOS simulator.")
    parser.add_argument("--udids", help="Two iOS UDIDs, comma/space separated. Overrides simulator names.")
    parser.add_argument("--udid-a", help="Bob iOS UDID.")
    parser.add_argument("--udid-b", help="Carol iOS UDID.")
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
        raise SystemExit("Need two iOS simulator names or UDIDs for mixed F14.")
    return {"simulator": simulators[0]}, {"simulator": simulators[1]}


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    if args.iterations < 1:
        raise SystemExit("--iterations must be at least 1")
    android_target = select_android_target(args)
    bob_ios, carol_ios = select_ios_entries(args)
    if args.relay_mode == "public":
        port = 0
        ios_url = args.relay_url or args.public_relays
        android_url = args.android_relay_url or args.relay_url or args.public_relays
        relay_config = {
            "start": False,
            "port": port,
            "set_id": f"mixed-multi-restart-soak-public-{artifact_dir.name}",
            "android_url": android_url,
            "url": ios_url,
        }
        reverse_relay = False
    else:
        port = args.relay_port or free_tcp_port()
        ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
        default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
        android_url = args.android_relay_url or args.relay_url or default_android_url
        relay_config = {
            "port": port,
            "label": f"iris.mixed-multi-restart-soak.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-multi-restart-soak-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        }
        reverse_relay = relay_uses_localhost(android_url)

    devices: list[dict[str, Any]] = [
        {
            "id": "alice1",
            "platform": "android",
            "run_id": "alice1",
            "user": "alice",
            "display_name": "Alice",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **android_target,
        },
        {
            "id": "bob1",
            "platform": "ios",
            "run_id": "bob1",
            "user": "bob",
            "display_name": "Bob",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **bob_ios,
        },
        {
            "id": "carol1",
            "platform": "ios",
            "run_id": "carol1",
            "user": "carol",
            "display_name": "Carol",
            "reset": True,
            "relay_drain_timeout_secs": int(USER_VISIBLE_TIMEOUT_SECS),
            **carol_ios,
        },
    ]

    config = {
        "name": f"mixed-multi-restart-soak-{artifact_dir.name}",
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
    config_path = artifact_dir / "mixed-multi-restart-soak-config.json"
    write_json(config_path, config)
    return config_path


def create_group(scenario: Scenario, flow_stamp: str) -> dict[str, str]:
    group_name = f"Mixed Restart Soak {flow_stamp}"
    statuses = harness(
        scenario,
        "alice1",
        "create_group_from_args",
        group_name=group_name,
        member_inputs=scenario.state["devices"]["bob1"]["owner_npub"],
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    group = {
        "name": group_name,
        "chat_id": statuses["chat_id"],
        "group_id": statuses["group_id"],
        "creator": "alice1",
    }
    scenario.state.setdefault("groups", {})["alice-bob-soak"] = group
    scenario.save_state()
    harness(scenario, "bob1", "wait_for_group_chat_from_args", chat_id=group["chat_id"], timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    for device_id in ("alice1", "bob1"):
        wait_member_count(scenario, device_id, group["chat_id"], 2)
    return group


def wait_member_count(scenario: Scenario, device_id: str, chat_id: str, count: int) -> str:
    return harness(
        scenario,
        device_id,
        "wait_for_group_member_count_from_args",
        chat_id=chat_id,
        member_count=str(count),
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    ).get("member_count", "")


def wait_group_name(scenario: Scenario, device_id: str, chat_id: str, group_name: str) -> str:
    return harness(
        scenario,
        device_id,
        "wait_for_group_name_from_args",
        chat_id=chat_id,
        group_name=group_name,
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    ).get("group_name", "")


def group_id_from_chat_id(chat_id: str) -> str:
    return chat_id.removeprefix("group:")


def add_carol(scenario: Scenario, chat_id: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "add_group_members_from_args",
        group_id=group_id_from_chat_id(chat_id),
        chat_id=chat_id,
        member_inputs=scenario.state["devices"]["carol1"]["owner_npub"],
        expected_member_count="3",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    counts = {"alice1": statuses.get("member_count", "")}
    for device_id in ("bob1", "carol1"):
        counts[device_id] = wait_member_count(scenario, device_id, chat_id, 3)
    return counts


def remove_carol(scenario: Scenario, chat_id: str) -> dict[str, str]:
    statuses = harness(
        scenario,
        "alice1",
        "remove_group_member_from_args",
        group_id=group_id_from_chat_id(chat_id),
        chat_id=chat_id,
        member_input=scenario.state["devices"]["carol1"]["owner_hex"],
        expected_member_count="2",
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    counts = {"alice1": statuses.get("member_count", "")}
    counts["bob1"] = wait_member_count(scenario, "bob1", chat_id, 2)
    return counts


def rename_group(scenario: Scenario, chat_id: str, group_name: str, members: list[str]) -> dict[str, str]:
    harness(
        scenario,
        "alice1",
        "update_group_name_from_args",
        group_id=group_id_from_chat_id(chat_id),
        chat_id=chat_id,
        group_name=group_name,
        wait_for_relay_drain="true",
        relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
    )
    return {device_id: wait_group_name(scenario, device_id, chat_id, group_name) for device_id in members}


def wait_direct_both_sides(
    scenario: Scenario,
    sender: str,
    receiver: str,
    message: str,
) -> dict[str, str]:
    return {
        sender: wait_peer(
            scenario,
            sender,
            receiver,
            message,
            direction="outgoing",
            expected_count=1,
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("matching_count", ""),
        receiver: wait_peer(
            scenario,
            receiver,
            sender,
            message,
            direction="incoming",
            expected_count=1,
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("matching_count", ""),
    }


def wait_group_on_members(
    scenario: Scenario,
    sender: str,
    chat_id: str,
    message: str,
    members: list[str],
) -> dict[str, str]:
    counts: dict[str, str] = {}
    for device_id in members:
        direction = "outgoing" if device_id == sender else "incoming"
        counts[device_id] = wait_chat(
            scenario,
            device_id,
            chat_id,
            message,
            direction=direction,
            expected_count=1,
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("matching_count", "")
    return counts


def restart_all(scenario: Scenario, members: list[str], *, wait_for_drain: bool = True) -> None:
    for device_id in members:
        restart_app(
            scenario,
            device_id,
            wait_for_drain=wait_for_drain,
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        )


def drain_all(scenario: Scenario, members: list[str]) -> None:
    for device_id in members:
        drain_after_restart(scenario, device_id, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)


def run_relay_outage_cycle(
    scenario: Scenario,
    chat_id: str,
    flow_stamp: str,
    iteration: int,
    members: list[str],
) -> dict[str, Any]:
    direct_message = f"soak-relay-offline-direct-{iteration}-{flow_stamp}"
    group_message = f"soak-relay-offline-group-{iteration}-{flow_stamp}"
    scenario.begin_fault()
    queued = {
        "direct": send_peer(
            scenario,
            "alice1",
            "bob1",
            direct_message,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("delivery", ""),
        "group": send_chat(
            scenario,
            "bob1",
            chat_id,
            group_message,
            wait_for_delivery=False,
            wait_for_relay_drain=False,
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("delivery", ""),
    }
    restart_all(scenario, members, wait_for_drain=False)
    scenario.start_relay()
    drain_all(scenario, members)
    return {
        "messages": {"direct": direct_message, "group": group_message},
        "queued_delivery": queued,
        "counts": {
            "direct": wait_direct_both_sides(scenario, "alice1", "bob1", direct_message),
            "group": wait_group_on_members(scenario, "bob1", chat_id, group_message, members),
        },
    }


def run_public_offline_catchup_cycle(
    scenario: Scenario,
    chat_id: str,
    flow_stamp: str,
    iteration: int,
    members: list[str],
) -> dict[str, Any]:
    direct_message = f"soak-public-offline-direct-{iteration}-{flow_stamp}"
    group_message = f"soak-public-offline-group-{iteration}-{flow_stamp}"
    offline_device = "bob1"
    online_members = [device_id for device_id in members if device_id != offline_device]

    stop_app(scenario, offline_device)
    published = {
        "direct": send_peer(
            scenario,
            "alice1",
            "bob1",
            direct_message,
            wait_for_delivery=False,
            wait_for_relay_drain=True,
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("delivery", ""),
        "group": send_chat(
            scenario,
            "alice1",
            chat_id,
            group_message,
            wait_for_delivery=False,
            wait_for_relay_drain=True,
            relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("delivery", ""),
    }
    counts: dict[str, Any] = {
        "direct_while_offline": {
            "alice1": wait_peer(
                scenario,
                "alice1",
                "bob1",
                direct_message,
                direction="outgoing",
                expected_count=1,
                timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
            ).get("matching_count", ""),
        },
        "group_while_offline": wait_group_on_members(scenario, "alice1", chat_id, group_message, online_members),
    }
    restart_app(scenario, offline_device, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    counts["direct_after_restart"] = {
        "bob1": wait_peer(
            scenario,
            "bob1",
            "alice1",
            direct_message,
            direction="incoming",
            expected_count=1,
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("matching_count", ""),
    }
    counts["group_after_restart"] = {
        "bob1": wait_chat(
            scenario,
            "bob1",
            chat_id,
            group_message,
            direction="incoming",
            expected_count=1,
            timeout_secs=USER_VISIBLE_TIMEOUT_SECS,
        ).get("matching_count", ""),
    }
    return {
        "messages": {"direct": direct_message, "group": group_message},
        "published_delivery": published,
        "offline_device": offline_device,
        "counts": counts,
    }


def run_iteration(
    scenario: Scenario,
    chat_id: str,
    flow_stamp: str,
    iteration: int,
    members: list[str],
) -> tuple[list[str], dict[str, Any]]:
    direct_a2b = f"soak-direct-a2b-{iteration}-{flow_stamp}"
    direct_b2a = f"soak-direct-b2a-{iteration}-{flow_stamp}"
    group_alice = f"soak-group-alice-{iteration}-{flow_stamp}"
    group_bob = f"soak-group-bob-{iteration}-{flow_stamp}"

    send_peer(scenario, "alice1", "bob1", direct_a2b, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    send_peer(scenario, "bob1", "alice1", direct_b2a, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    send_chat(scenario, "alice1", chat_id, group_alice, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    send_chat(scenario, "bob1", chat_id, group_bob, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
    persistence_sender = "bob1"
    persistence_message = group_bob

    result: dict[str, Any] = {
        "messages": {
            "direct_a2b": direct_a2b,
            "direct_b2a": direct_b2a,
            "group_alice": group_alice,
            "group_bob": group_bob,
        },
        "counts": {
            "direct_a2b": wait_direct_both_sides(scenario, "alice1", "bob1", direct_a2b),
            "direct_b2a": wait_direct_both_sides(scenario, "bob1", "alice1", direct_b2a),
            "group_alice": wait_group_on_members(scenario, "alice1", chat_id, group_alice, members),
            "group_bob": wait_group_on_members(scenario, "bob1", chat_id, group_bob, members),
        },
    }

    if iteration == 1 and "carol1" not in members:
        result["add_carol_counts"] = add_carol(scenario, chat_id)
        members = [*members, "carol1"]
        renamed = f"Mixed Restart Soak {flow_stamp} + Carol"
        result["rename_with_carol"] = rename_group(scenario, chat_id, renamed, members)
        carol_message = f"soak-group-carol-{iteration}-{flow_stamp}"
        send_chat(scenario, "carol1", chat_id, carol_message, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
        result["messages"]["group_carol"] = carol_message
        result["counts"]["group_carol"] = wait_group_on_members(scenario, "carol1", chat_id, carol_message, members)
        persistence_sender = "carol1"
        persistence_message = carol_message

    if iteration == 2 and "carol1" in members:
        result["remove_carol_counts"] = remove_carol(scenario, chat_id)
        members = [device_id for device_id in members if device_id != "carol1"]
        renamed = f"Mixed Restart Soak {flow_stamp} Final"
        result["rename_after_remove"] = rename_group(scenario, chat_id, renamed, members)
        post_remove = f"soak-group-post-remove-{iteration}-{flow_stamp}"
        send_chat(scenario, "alice1", chat_id, post_remove, relay_drain_timeout_secs=USER_VISIBLE_TIMEOUT_SECS)
        result["messages"]["group_post_remove"] = post_remove
        result["counts"]["group_post_remove"] = wait_group_on_members(scenario, "alice1", chat_id, post_remove, members)
        persistence_sender = "alice1"
        persistence_message = post_remove

    restart_all(scenario, members)
    result["post_restart_counts"] = wait_group_on_members(scenario, persistence_sender, chat_id, persistence_message, members)
    if scenario.uses_local_relay():
        result["relay_outage"] = run_relay_outage_cycle(scenario, chat_id, flow_stamp, iteration, members)
    else:
        result["public_offline_catchup"] = run_public_offline_catchup_cycle(
            scenario,
            chat_id,
            flow_stamp,
            iteration,
            members,
        )
    return members, result


def run_flow(scenario: Scenario, artifact_dir: Path, iterations: int) -> dict[str, Any]:
    flow_stamp = short_stamp()
    group = create_group(scenario, flow_stamp)
    chat_id = group["chat_id"]
    members = ["alice1", "bob1"]

    iterations_result: list[dict[str, Any]] = []
    for iteration in range(1, iterations + 1):
        members, result = run_iteration(scenario, chat_id, flow_stamp, iteration, members)
        iterations_result.append(result)

    for device_id in ("alice1", "bob1", "carol1"):
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
        "relay_mode": "local" if scenario.uses_local_relay() else "public",
        "relay_urls": scenario.relay_url(),
        "android_relay_urls": scenario.android_relay_url(),
        "iterations": iterations,
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "final_members": members,
        "iteration_results": iterations_result,
        "harness_action_history": scenario.action_history,
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
    write_json(artifact_dir / "mixed-multi-restart-soak-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (
        args.artifact_dir or Path(f"/tmp/iris-mixed-multi-restart-soak-{args.relay_mode}-{run_id}")
    ).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        try:
            scenario.setup()
            result = run_flow(scenario, artifact_dir, args.iterations)
            print(json.dumps(result, indent=2, sort_keys=True))
        except BaseException as exc:
            failure = {
                "status": "failed",
                "artifact_dir": str(artifact_dir),
                "error": str(exc),
                "error_type": type(exc).__name__,
                "harness_action_history": scenario.action_history,
                "state": str(scenario.state_path),
            }
            write_json(artifact_dir / "mixed-multi-restart-soak-summary.json", failure)
            raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
