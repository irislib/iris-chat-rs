#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

from android_offline_restart_recovery import (
    build_config as build_android_config,
    harness,
    send_chat,
    stamp,
    wait_chat,
    write_json,
)
from mobile_scenario import Scenario


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Android cold group invite E2E.")
    parser.add_argument("--artifact-dir", type=Path, help="Directory for generated config, state, and summary.")
    parser.add_argument("--headless", action="store_true", help="Launch Android emulators headlessly.")
    parser.add_argument("--wipe-data", action="store_true", help="Wipe AVD data before launch.")
    parser.add_argument("--skip-build", action="store_true", help="Reuse installed APKs. Requires matching local relay URL.")
    parser.add_argument("--keep-devices-open", action="store_true", help="Leave emulators running after the flow.")
    parser.add_argument("--relay-port", type=int, help="Local relay TCP port. Default: random free port.")
    parser.add_argument("--relay-url", help="Relay URL compiled into the Android debug app.")
    parser.add_argument("--serials", help="One or two adb serials, space-separated. Can be combined with --avds.")
    parser.add_argument("--avds", help="One or two AVD names, space-separated. Can be combined with --serials.")
    return parser.parse_args()


def retag_config(config_path: Path, artifact_dir: Path) -> None:
    config = json.loads(config_path.read_text(encoding="utf-8"))
    config["name"] = f"android-cold-group-{artifact_dir.name}"
    relay = config.get("relay", {})
    relay["label"] = f"iris.android-cold-group.{artifact_dir.name}.relay"
    relay["set_id"] = f"android-cold-group-{artifact_dir.name}"
    config["relay"] = relay
    config_path.write_text(json.dumps(config, indent=2, sort_keys=True) + "\n", encoding="utf-8")


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


def run_flow(scenario: Scenario, artifact_dir: Path) -> dict[str, Any]:
    flow_stamp = time.strftime("%H%M%S")
    group = create_cold_group(
        scenario,
        f"Android Cold Group {artifact_dir.name[-6:]}",
    )
    chat_id = group["chat_id"]
    messages = {
        "alice_group": f"android-cold-group-alice-{flow_stamp}",
        "bob_group": f"android-cold-group-bob-{flow_stamp}",
    }

    send_chat(scenario, "alice1", chat_id, messages["alice_group"])
    bob_count = wait_chat(
        scenario,
        "bob1",
        chat_id,
        messages["alice_group"],
        expected_count=1,
    ).get("matching_count", "")
    send_chat(scenario, "bob1", chat_id, messages["bob_group"])
    alice_count = wait_chat(
        scenario,
        "alice1",
        chat_id,
        messages["bob_group"],
        expected_count=1,
    ).get("matching_count", "")

    for device_id in ("alice1", "bob1"):
        harness(scenario, device_id, "report_runtime_debug_snapshot")
        harness(scenario, device_id, "report_persisted_protocol_snapshot")

    result = {
        "status": "passed",
        "artifact_dir": str(artifact_dir),
        "group_chat_id": chat_id,
        "group_id": group["group_id"],
        "messages": messages,
        "duplicate_counts": {
            "bob_group_from_alice": bob_count,
            "alice_group_from_bob": alice_count,
        },
        "state": str(scenario.state_path),
    }
    write_json(artifact_dir / "android-cold-group-summary.json", result)
    return result


def main() -> int:
    args = parse_args()
    run_id = stamp()
    artifact_dir = (args.artifact_dir or Path(f"/tmp/iris-android-cold-group-{run_id}")).resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_path = build_android_config(args, artifact_dir)
    retag_config(config_path, artifact_dir)
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
