#!/usr/bin/env python3
"""Run reproducible Rust core timing probes for this branch and experimental-chat.

The probes print `CORE_TIMING_DATASET` JSON lines from Rust tests. This runner
captures the raw cargo logs and emits a combined JSONL dataset under
`target/protocol-timing/`.
"""

from __future__ import annotations

import datetime as _dt
import json
import os
import pathlib
import re
import subprocess
import sys


MARKER = "CORE_TIMING_DATASET "


def repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[1]


def default_experimental_root(current_root: pathlib.Path) -> pathlib.Path:
    return current_root.parents[1] / "iris-fork" / "experimental-chat" / "iris-chat-rs"


def cargo_env() -> dict[str, str]:
    env = os.environ.copy()
    env.setdefault("IRIS_APP_VERSION", "timing-harness")
    env.setdefault("IRIS_BUILD_CHANNEL", "timing")
    env.setdefault("IRIS_BUILD_GIT_SHA", "timing")
    env.setdefault("IRIS_BUILD_TIMESTAMP_UTC", "1970-01-01T00:00:00Z")
    env.setdefault("IRIS_DEFAULT_RELAYS", "ws://127.0.0.1:4848")
    env.setdefault("IRIS_RELAY_SET_ID", "timing")
    env.setdefault("IRIS_TRUSTED_TEST_BUILD", "1")
    env.setdefault("NDR_APP_VERSION", "timing-harness")
    env.setdefault("NDR_BUILD_CHANNEL", "timing")
    env.setdefault("NDR_BUILD_GIT_SHA", "timing")
    env.setdefault("NDR_BUILD_TIMESTAMP_UTC", "1970-01-01T00:00:00Z")
    return env


def run_probe(label: str, root: pathlib.Path, test_name: str, out_dir: pathlib.Path) -> list[dict]:
    cmd = [
        "cargo",
        "test",
        "--manifest-path",
        str(root / "core" / "Cargo.toml"),
        test_name,
        "--",
        "--nocapture",
    ]
    print(f"running {label}: {' '.join(cmd)}", flush=True)
    proc = subprocess.run(
        cmd,
        cwd=root,
        env=cargo_env(),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    log_path = out_dir / f"{label}.log"
    log_path.write_text(proc.stdout)
    if proc.returncode != 0:
        print(proc.stdout)
        raise SystemExit(f"{label} failed with exit code {proc.returncode}; log={log_path}")

    records: list[dict] = []
    for line in proc.stdout.splitlines():
        if MARKER not in line:
            continue
        payload = line.split(MARKER, 1)[1].strip()
        try:
            record = json.loads(payload)
        except json.JSONDecodeError as exc:
            raise SystemExit(f"failed to parse timing payload from {label}: {payload}") from exc
        record["probe"] = label
        record["repo_path"] = str(root)
        records.append(record)
    if not records:
        raise SystemExit(f"{label} produced no {MARKER.strip()} records; log={log_path}")
    return records


def main() -> int:
    current = repo_root()
    experimental = pathlib.Path(
        os.environ.get("IRIS_EXPERIMENTAL_CHAT_RS", default_experimental_root(current))
    ).resolve()
    if not experimental.exists():
        raise SystemExit(
            "experimental-chat repo not found; set IRIS_EXPERIMENTAL_CHAT_RS=/path/to/iris-chat-rs"
        )

    stamp = _dt.datetime.now(_dt.UTC).strftime("%Y%m%dT%H%M%SZ")
    out_dir = current / "target" / "protocol-timing" / stamp
    out_dir.mkdir(parents=True, exist_ok=True)

    probes = [
        (
            "current",
            current,
            "timing_current_publish_waits_for_slow_first_relay",
        ),
        (
            "experimental",
            experimental,
            "timing_experimental_publish_returns_on_fast_first_ack",
        ),
    ]
    records: list[dict] = []
    for label, root, test_name in probes:
        records.extend(run_probe(label, root, test_name, out_dir))

    dataset_path = out_dir / "core_timing_dataset.jsonl"
    dataset_path.write_text("".join(json.dumps(record, sort_keys=True) + "\n" for record in records))
    summary_path = out_dir / "summary.txt"
    summary = ["Core timing dataset", f"output_dir={out_dir}", ""]
    for record in records:
        summary.append(
            "{repo} {strategy} scenario={scenario} elapsed_ms={elapsed_ms}".format(**record)
        )
    summary_path.write_text("\n".join(summary) + "\n")
    print(summary_path.read_text())
    print(f"dataset={dataset_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
