#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "scripts"
DEFAULT_CONFIG = SCRIPTS / "scenarios" / "alice_alice2_bob_group.json"
IOS_HARNESS = SCRIPTS / "run_ios_harness.py"
MOBILE_SCENARIO = SCRIPTS / "mobile_scenario.py"
PENDING_PUBLISHES = SCRIPTS / "pending_relay_publishes.py"
STATUS_PATTERN = re.compile(r"^INSTRUMENTATION_STATUS: ([^=]+)=(.*)$")


def run(
    command: list[str],
    *,
    cwd: Path = ROOT,
    log_path: Path | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text("+ " + " ".join(command) + "\n" + completed.stdout, encoding="utf-8")
    if check and completed.returncode != 0:
        sys.stdout.write(completed.stdout)
        raise SystemExit(completed.returncode)
    return completed


def parse_status(output: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in output.splitlines():
        match = STATUS_PATTERN.match(line.strip())
        if match:
            key, value = match.groups()
            values[key.lower()] = value
    return values


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def scenario_work_dir(config: Path) -> Path:
    config_data = load_json(config)
    work_dir = Path(config_data["work_dir"])
    return work_dir


def load_state(config: Path) -> dict[str, Any]:
    state_path = scenario_work_dir(config) / "state.json"
    if not state_path.exists():
        raise SystemExit(f"Scenario state not found at {state_path}. Run with --setup first.")
    return load_json(state_path)


def scenario_command(config: Path, command: str, *extra: str, log_path: Path | None = None) -> None:
    run(
        [sys.executable, str(MOBILE_SCENARIO), "--config", str(config), command, *extra],
        log_path=log_path,
    )


def start_relay(config: Path, log_path: Path) -> None:
    # Reuse the scenario runtime so the launchctl label, drop file, and local relay
    # binary resolution stay identical to the normal mobile scenario flow.
    code = (
        "from pathlib import Path\n"
        "import sys\n"
        f"sys.path.insert(0, {str(SCRIPTS)!r})\n"
        "from mobile_scenario import Scenario\n"
        f"Scenario(Path({str(config)!r})).start_relay()\n"
    )
    run([sys.executable, "-c", code], log_path=log_path)


def harness(
    state: dict[str, Any],
    work_dir: Path,
    plaintext_log: Path,
    device_id: str,
    action: str,
    *,
    args: dict[str, str] | None = None,
    check: bool = True,
    log_suffix: str | None = None,
) -> subprocess.CompletedProcess[str]:
    device = state["devices"][device_id]
    if device["platform"] != "ios":
        raise SystemExit("This revision repair reproduction currently uses the iOS harness only.")
    command = [
        sys.executable,
        str(IOS_HARNESS),
        "--udid",
        device["udid"],
        "--use-app-storage",
        "--run-id",
        device["run_id"],
        "--action",
        action,
        "--arg",
        f"protocol_plaintext_log_file={plaintext_log}",
    ]
    for key, value in (args or {}).items():
        command.extend(["--arg", f"{key}={value}"])
    suffix = log_suffix or action
    log_path = work_dir / f"revision-repair-{device_id}-{suffix}.log"
    completed = run(command, log_path=log_path, check=False)
    ok = completed.returncode == 0 and "INSTRUMENTATION_CODE: -1" in completed.stdout
    if check and not ok:
        sys.stdout.write(completed.stdout)
        raise SystemExit(completed.returncode or 1)
    return completed


def status_summary(completed: subprocess.CompletedProcess[str]) -> dict[str, str]:
    values = parse_status(completed.stdout)
    for key in sorted(values):
        if key in {"action", "chat_id", "group_id", "group_name", "message", "network_connected_relay_count"}:
            print(f"  {key}={values[key]}")
    return values


def ensure_localhost_relay(state: dict[str, Any], work_dir: Path, plaintext_log: Path, port: int) -> None:
    relay_url = f"ws://127.0.0.1:{port}"
    for device_id in ("alice1", "alice2", "bob1"):
        print(f"Adding localhost relay to {device_id}: {relay_url}")
        status_summary(
            harness(
                state,
                work_dir,
                plaintext_log,
                device_id,
                "add_relay_from_args",
                args={"relay_url": relay_url},
                log_suffix="add-localhost-relay",
            )
        )
        status_summary(
            harness(
                state,
                work_dir,
                plaintext_log,
                device_id,
                "wait_for_connected_relay",
                args={"timeout_secs": "45"},
                log_suffix="wait-connected-localhost",
            )
        )


def list_pending_pairwise_for_target(
    data_dir: str,
    target_owner_hex: str,
    target_device_hex: str,
    output_path: Path,
) -> list[dict[str, Any]]:
    completed = run(
        [
            sys.executable,
            str(PENDING_PUBLISHES),
            "list",
            "--data-dir",
            data_dir,
            "--target-owner-hex",
            target_owner_hex,
            "--target-device-hex",
            target_device_hex,
            "--pairwise-only",
            "--format",
            "json",
        ],
        log_path=output_path.with_suffix(".log"),
    )
    output_path.write_text(completed.stdout, encoding="utf-8")
    return json.loads(completed.stdout)


def select_newest_pending_row(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        raise SystemExit("No Bob-targeted pairwise pending publish was found to drop.")
    return sorted(rows, key=lambda row: (row.get("created_at_secs") or 0, row["event_id"]))[-1]


def parse_plaintext_log(path: Path, group_id: str) -> dict[str, Any]:
    rows = []
    if path.exists():
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            payload = item.get("payload") or {}
            if payload.get("group_id") == group_id:
                rows.append(payload)
    counts = Counter(row.get("type") or "sender_key_plaintext_event" for row in rows)
    repair_requests = [row for row in rows if row.get("type") == "sender_key_repair_request"]
    metadata_snapshots = [row for row in rows if row.get("type") == "metadata_snapshot"]
    sender_key_messages = [row for row in rows if row.get("type") is None]
    return {
        "counts": dict(counts),
        "repair_requests": repair_requests,
        "metadata_snapshots": metadata_snapshots,
        "sender_key_messages": sender_key_messages,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Reproduce sender-key metadata revision repair with a local relay exact drop. "
            "Requires a build whose group codec honors protocol_plaintext_log_file."
        )
    )
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--setup", action="store_true", help="Run scripts/mobile_scenario.py setup first.")
    parser.add_argument("--group-key", default="alice-bob")
    parser.add_argument("--skip-baseline", action="store_true")
    parser.add_argument(
        "--skip-force-activation",
        action="store_true",
        help="Stop after the passive Bob wait. Useful when checking whether liveness recovers by itself.",
    )
    args = parser.parse_args()

    config = args.config.resolve()
    work_dir = scenario_work_dir(config)
    work_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%H%M%S")
    plaintext_log = work_dir / f"protocol-plaintext-revision-{stamp}.log"
    plaintext_log.write_text("", encoding="utf-8")

    if args.setup:
        scenario_command(config, "setup", log_path=work_dir / f"revision-repair-setup-{stamp}.log")

    state = load_state(config)
    group = state["groups"][args.group_key]
    relay = state["relay"]
    alice = state["devices"]["alice1"]
    alice2 = state["devices"]["alice2"]
    bob = state["devices"]["bob1"]

    start_relay(config, work_dir / f"revision-repair-start-relay-{stamp}.log")
    ensure_localhost_relay(state, work_dir, plaintext_log, int(relay["port"]))

    baseline_message = f"revision-baseline-{stamp}"
    new_name = f"Revision Repair {stamp}"
    repair_message = f"revision-repair-message-{stamp}"
    (work_dir / "current-revision-name.txt").write_text(new_name + "\n", encoding="utf-8")
    (work_dir / "current-revision-message.txt").write_text(repair_message + "\n", encoding="utf-8")

    if not args.skip_baseline:
        print("Sending baseline group message so Bob definitely knows Alice's sender-key author.")
        status_summary(
            harness(
                state,
                work_dir,
                plaintext_log,
                "alice1",
                "send_message_from_args",
                args={
                    "chat_id": group["chat_id"],
                    "message": baseline_message,
                    "wait_for_relay_drain": "true",
                    "relay_drain_timeout_secs": "180",
                },
                log_suffix="baseline-send",
            )
        )
        status_summary(
            harness(
                state,
                work_dir,
                plaintext_log,
                "bob1",
                "wait_for_message_from_args",
                args={
                    "chat_id": group["chat_id"],
                    "message": baseline_message,
                    "direction": "incoming",
                },
                log_suffix="baseline-wait-bob",
            )
        )

    print("Stopping relay and renaming group offline.")
    scenario_command(config, "begin-fault", log_path=work_dir / f"revision-repair-begin-fault-{stamp}.log")
    status_summary(
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice1",
            "update_group_name_from_args",
            args={
                "group_id": group["group_id"],
                "group_name": new_name,
                "wait_for_relay_drain": "false",
            },
            log_suffix="offline-rename",
        )
    )

    pending_rows = list_pending_pairwise_for_target(
        alice["data_dir"],
        bob["owner_hex"],
        bob["device_hex"],
        work_dir / f"revision-repair-bob-pending-before-drop-{stamp}.json",
    )
    drop_row = select_newest_pending_row(pending_rows)
    drop_file = Path(relay["drop_file"])
    drop_file.write_text(drop_row["event_id"] + "\n", encoding="utf-8")
    (work_dir / f"revision-repair-drop-id-{stamp}.txt").write_text(
        drop_row["event_id"] + "\n", encoding="utf-8"
    )
    print(f"Dropping Bob metadata/control event: {drop_row['event_id']}")

    print("Restarting relay and confirming Alice's linked device sees the rename.")
    start_relay(config, work_dir / f"revision-repair-restart-relay-after-drop-{stamp}.log")
    # The offline rename lives in Alice's durable pending publish queue. The
    # iOS harness runs one action at a time, so explicitly activate Alice after
    # the relay restart before asking Alice2 to observe the rename. Without this
    # step, the test can stall before it reaches the intended Bob revision gap.
    status_summary(
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice1",
            "wait_for_connected_relay",
            args={"timeout_secs": "45"},
            log_suffix="alice-wait-connected-after-rename-drop",
        )
    )
    time.sleep(2)
    status_summary(
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice2",
            "wait_for_group_name_from_args",
            args={"chat_id": group["chat_id"], "group_name": new_name},
            log_suffix="alice2-wait-renamed",
        )
    )

    print("Sending group message that requires Bob's missed revision.")
    status_summary(
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice1",
            "send_message_from_args",
            args={
                "chat_id": group["chat_id"],
                "message": repair_message,
                "wait_for_relay_drain": "true",
                "relay_drain_timeout_secs": "180",
            },
            log_suffix="send-revision-message",
        )
    )

    print("Passive Bob wait: this records whether the repair converges without touching Alice/Bob again.")
    passive = harness(
        state,
        work_dir,
        plaintext_log,
        "bob1",
        "wait_for_message_from_args",
        args={"chat_id": group["chat_id"], "message": repair_message, "direction": "incoming"},
        check=False,
        log_suffix="bob-passive-wait-message",
    )
    passive_success = passive.returncode == 0 and "INSTRUMENTATION_CODE: -1" in passive.stdout
    print(f"Passive Bob wait success: {passive_success}")

    forced_success = False
    if not passive_success and not args.skip_force_activation:
        print("Activating both sides and forcing connected-relay/liveness work.")
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice1",
            "report_runtime_debug_snapshot",
            log_suffix="alice-activate-after-passive-timeout",
        )
        harness(
            state,
            work_dir,
            plaintext_log,
            "alice1",
            "wait_for_connected_relay",
            args={"timeout_secs": "45"},
            log_suffix="alice-wait-connected-after-passive-timeout",
        )
        harness(
            state,
            work_dir,
            plaintext_log,
            "bob1",
            "wait_for_connected_relay",
            args={"timeout_secs": "45"},
            log_suffix="bob-wait-connected-after-passive-timeout",
        )
        time.sleep(5)
        forced = harness(
            state,
            work_dir,
            plaintext_log,
            "bob1",
            "wait_for_message_from_args",
            args={"chat_id": group["chat_id"], "message": repair_message, "direction": "incoming"},
            check=False,
            log_suffix="bob-forced-wait-message",
        )
        forced_success = forced.returncode == 0 and "INSTRUMENTATION_CODE: -1" in forced.stdout
        if forced_success:
            status_summary(
                harness(
                    state,
                    work_dir,
                    plaintext_log,
                    "bob1",
                    "wait_for_group_name_from_args",
                    args={"chat_id": group["chat_id"], "group_name": new_name},
                    log_suffix="bob-forced-wait-name",
                )
            )
        print(f"Forced activation success: {forced_success}")

    parsed = parse_plaintext_log(plaintext_log, group["group_id"])
    summary = {
        "stamp": stamp,
        "group_id": group["group_id"],
        "group_chat_id": group["chat_id"],
        "new_name": new_name,
        "repair_message": repair_message,
        "dropped_event_id": drop_row["event_id"],
        "plaintext_log": str(plaintext_log),
        "passive_success": passive_success,
        "forced_success": forced_success,
        "repair_request_count": len(parsed["repair_requests"]),
        "metadata_snapshot_count": len(parsed["metadata_snapshots"]),
        "plaintext_counts": parsed["counts"],
    }
    summary_path = work_dir / f"revision-repair-summary-{stamp}.json"
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2, sort_keys=True))

    if summary["repair_request_count"] == 0:
        print("No sender_key_repair_request was observed in the plaintext log.", file=sys.stderr)
        return 2
    if not (passive_success or forced_success):
        print("Repair request was observed, but Bob did not apply the message.", file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
