#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
from pathlib import Path
from typing import Any

from mixed_offline_restart_recovery import (
    connected_android_serials,
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
    wait_chat,
    wait_peer,
    write_json,
)
from mobile_scenario import (
    ROOT_DIR,
    Scenario,
    parse_status,
    redact_sensitive_text,
    run,
    wait_for_status_in_files,
)


DEFAULT_SIMULATORS = ("Iris Chat iPhone", "Iris Chat iPhone 2")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mixed iOS/Android link bootstrap restart E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed artifacts. Requires matching local relay URLs.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave simulator/emulator windows running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local message-server TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="URL compiled into the iOS harness. Default: ws://127.0.0.1:<port>.")
    parser.add_argument("--android-relay-url", help="URL compiled into the Android debug app.")
    parser.add_argument("--serial", help="ADB serial for the Android linked device.")
    parser.add_argument("--serials", help="ADB serials, space/comma separated. First value is used.")
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
            f"Need {limit} Android AVD or connected Android device for mixed F03; found {len(avds)} AVDs. "
            "Set --serial, --avd, IRIS_ANDROID_E2E_SERIALS, or IRIS_ANDROID_E2E_AVDS."
        )
    return avds[:limit]


def select_android_target(args: argparse.Namespace) -> dict[str, str]:
    serials = []
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
        raise SystemExit("Need two iOS simulator names or UDIDs for mixed F03.")
    return {"simulator": simulators[0]}, {"simulator": simulators[1]}


def build_config(args: argparse.Namespace, artifact_dir: Path) -> Path:
    port = args.relay_port or free_tcp_port()
    ios_url = args.relay_url or f"ws://127.0.0.1:{port}"
    android_target = select_android_target(args)
    alice_ios, bob_ios = select_ios_entries(args)
    default_android_url = f"ws://127.0.0.1:{port}" if "serial" in android_target else f"ws://10.0.2.2:{port}"
    android_url = args.android_relay_url or args.relay_url or default_android_url

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
        "name": f"mixed-link-restart-{artifact_dir.name}",
        "work_dir": str(artifact_dir / "scenario"),
        "relay": {
            "port": port,
            "label": f"iris.mixed-link-restart.{artifact_dir.name}.relay",
            "drop_file": str(artifact_dir / "scenario" / "drop-events.txt"),
            "log_file": str(artifact_dir / "scenario" / "relay.log"),
            "set_id": f"mixed-link-restart-{artifact_dir.name}",
            "bind_host": "0.0.0.0",
            "android_url": android_url,
            "url": ios_url,
        },
        "ios": {"build": not args.skip_build},
        "android": {
            "build": not args.skip_build,
            "headless": args.headless,
            "wipe_data": args.wipe_data,
            "reverse_relay": relay_uses_localhost(android_url),
        },
        "open_apps": True,
        "devices": devices,
        "groups": [],
    }
    config_path = artifact_dir / "mixed-link-restart-config.json"
    write_json(config_path, config)
    return config_path


def setup_without_linking(scenario: Scenario) -> None:
    scenario.work_dir.mkdir(parents=True, exist_ok=True)
    scenario.start_relay()
    scenario.boot_ios()
    scenario.boot_android()
    scenario.build_ios()
    scenario.build_android()
    scenario.configure_android_relay_access()
    rebuild_next_ios = bool(scenario.config.get("ios", {}).get("build", True))
    for device in scenario.config.get("devices", []):
        if device.get("linked_to"):
            continue
        rebuild = bool(device.get("platform") == "ios" and rebuild_next_ios)
        scenario.create_account(device, rebuild=rebuild)
        if device.get("platform") == "ios":
            rebuild_next_ios = False
    scenario.open_apps()
    scenario.save_state()


def link_without_owner_drain(scenario: Scenario, device_id: str = "alice2") -> dict[str, Any]:
    device_config = next(device for device in scenario.config["devices"] if device["id"] == device_id)
    owner_user = device_config["linked_to"]
    owner_device_id = scenario.primary_device_for_user(owner_user)
    owner = scenario.state["users"].get(owner_user)
    if not owner:
        raise SystemExit(f"Cannot link {device_id}; owner user {owner_user} has no identity")

    status_file = scenario.work_dir / f"{device_id}-link.status"
    log_file = scenario.work_dir / f"{device_id}-link.log"
    status_file.unlink(missing_ok=True)
    with log_file.open("w", encoding="utf-8") as handle:
        command = scenario.harness_command(
            device_id,
            scenario.link_wait_action(device_id),
            args=scenario.link_wait_args(device_id, owner["npub"], status_file),
            reset=bool(device_config.get("reset", False)),
        )
        print("+ " + " ".join(shlex.quote(part) for part in command), flush=True)
        process = subprocess.Popen(
            command,
            cwd=str(ROOT_DIR),
            env=scenario.scenario_env(),
            stdout=handle,
            stderr=subprocess.STDOUT,
            text=True,
        )

    link_url = wait_for_status_in_files(
        [status_file, log_file],
        scenario.link_status_key(device_id),
        int(device_config.get("link_timeout_secs", 180)),
    )
    owner_add = scenario.harness(
        owner_device_id,
        "add_authorized_device_from_args",
        args={"device_input": link_url},
    )
    exit_code = process.wait(timeout=int(device_config.get("authorization_timeout_secs", 300)))
    output = log_file.read_text(encoding="utf-8", errors="replace")
    if exit_code != 0 or "INSTRUMENTATION_CODE: -1" not in output:
        print(redact_sensitive_text(output))
        raise SystemExit(f"Linked device authorization failed for {device_id}")

    status_output = ""
    if status_file.exists():
        status_output = status_file.read_text(encoding="utf-8", errors="replace")
    linked_status = parse_status(output + "\n" + status_output)
    scenario.record_identity(device_id, linked_status)
    return {
        "owner_add": owner_add,
        "linked_status": {
            key: value
            for key, value in linked_status.items()
            if key not in {"invite_url", "link_url"}
        },
    }


def create_group_after_restart(scenario: Scenario, group_name: str) -> dict[str, str]:
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
    scenario.state.setdefault("groups", {})["alice-bob-link-restart"] = group_state
    scenario.save_state()
    harness(scenario, "bob1", "wait_for_group_chat_from_args", chat_id=group_state["chat_id"], timeout_secs="300")
    harness(scenario, "alice2", "wait_for_group_chat_from_args", chat_id=group_state["chat_id"], timeout_secs="300")
    return group_state


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = short_stamp()
    setup_without_linking(scenario)
    link_result = link_without_owner_drain(scenario)

    restart_app(scenario, "alice1", wait_for_drain=False)
    restart_app(scenario, "alice2", wait_for_drain=False)
    restart_app(scenario, "bob1", wait_for_drain=False)
    restarted = {
        "alice1": drain_after_restart(scenario, "alice1"),
        "alice2": drain_after_restart(scenario, "alice2"),
        "bob1": drain_after_restart(scenario, "bob1"),
    }
    alice = scenario.state["devices"]["alice1"]
    alice_linked = scenario.state["devices"]["alice2"]
    bob = scenario.state["devices"]["bob1"]
    bob_owner_hex = bob["owner_hex"]

    primary_message = f"mixed-link-restart-primary-{flow_stamp}"
    send_peer(scenario, "alice1", "bob1", primary_message)
    primary_counts = {
        "bob1": wait_peer(scenario, "bob1", "alice1", primary_message, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice2": wait_chat(scenario, "alice2", bob_owner_hex, primary_message, direction="outgoing", expected_count=1).get("matching_count", ""),
    }

    bob_reply = f"mixed-link-restart-reply-{flow_stamp}"
    send_peer(scenario, "bob1", "alice1", bob_reply)
    reply_counts = {
        "alice1": wait_peer(scenario, "alice1", "bob1", bob_reply, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice2": wait_chat(scenario, "alice2", bob_owner_hex, bob_reply, direction="incoming", expected_count=1).get("matching_count", ""),
    }

    linked_message = f"mixed-link-restart-linked-{flow_stamp}"
    send_peer(scenario, "alice2", "bob1", linked_message)
    linked_counts = {
        "bob1": wait_peer(scenario, "bob1", "alice1", linked_message, direction="incoming", expected_count=1).get("matching_count", ""),
        "alice1": wait_chat(scenario, "alice1", bob_owner_hex, linked_message, direction="outgoing", expected_count=1).get("matching_count", ""),
    }

    group = create_group_after_restart(scenario, f"Link restart {flow_stamp}")
    chat_id = group["chat_id"]
    group_message = f"mixed-link-restart-group-{flow_stamp}"
    send_chat(scenario, "alice1", chat_id, group_message)
    group_counts = {
        "alice1": wait_chat(scenario, "alice1", chat_id, group_message, direction="outgoing", expected_count=1).get("matching_count", ""),
        "alice2": wait_chat(scenario, "alice2", chat_id, group_message, direction="outgoing", expected_count=1).get("matching_count", ""),
        "bob1": wait_chat(scenario, "bob1", chat_id, group_message, direction="incoming", expected_count=1).get("matching_count", ""),
    }

    linked_group_message = f"mixed-link-restart-linked-group-{flow_stamp}"
    send_chat(scenario, "alice2", chat_id, linked_group_message)
    linked_group_counts = {
        "alice1": wait_chat(scenario, "alice1", chat_id, linked_group_message, direction="outgoing", expected_count=1).get("matching_count", ""),
        "alice2": wait_chat(scenario, "alice2", chat_id, linked_group_message, direction="outgoing", expected_count=1).get("matching_count", ""),
        "bob1": wait_chat(scenario, "bob1", chat_id, linked_group_message, direction="incoming", expected_count=1).get("matching_count", ""),
    }

    snapshots: dict[str, dict[str, dict[str, str]]] = {}
    for device_id in ("alice1", "alice2", "bob1"):
        snapshots[device_id] = {
            "runtime": harness(scenario, device_id, "report_runtime_debug_snapshot"),
            "persisted": harness(scenario, device_id, "report_persisted_protocol_snapshot"),
        }

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "link_result": link_result,
        "restarted": restarted,
        "primary_message": primary_message,
        "primary_counts": primary_counts,
        "bob_reply": bob_reply,
        "reply_counts": reply_counts,
        "linked_message": linked_message,
        "linked_counts": linked_counts,
        "group": group,
        "group_message": group_message,
        "group_counts": group_counts,
        "linked_group_message": linked_group_message,
        "linked_group_counts": linked_group_counts,
        "snapshots": snapshots,
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
    write_json(artifact_dir / "mixed-link-restart-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-mixed-link-restart-{run_id}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_config(args, artifact_dir)
    scenario = Scenario(config_path)
    try:
        result = run_flow(scenario, artifact_dir)
        print(json.dumps(result, indent=2, sort_keys=True))
    except BaseException as error:
        failure = {
            "status": "failed",
            "artifact_dir": str(artifact_dir),
            "error": str(error),
        }
        write_json(artifact_dir / "mixed-link-restart-summary.json", failure)
        raise
    finally:
        scenario.cleanup(shutdown_devices=not args.keep_devices_open)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
