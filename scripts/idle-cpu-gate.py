#!/usr/bin/env python3
"""Fail when an Iris Chat process burns CPU after its fixture becomes idle."""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


def generated_at() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def parse_ps_time(raw: str) -> float:
    value = raw.strip()
    if not value:
        raise ValueError("empty ps time")
    days = 0
    if "-" in value:
        day_part, value = value.split("-", 1)
        days = int(day_part)
    parts = value.split(":")
    if len(parts) == 3:
        hours, minutes, seconds = int(parts[0]), int(parts[1]), float(parts[2])
    elif len(parts) == 2:
        hours, minutes, seconds = 0, int(parts[0]), float(parts[1])
    elif len(parts) == 1:
        hours, minutes, seconds = 0, 0, float(parts[0])
    else:
        raise ValueError(f"unsupported ps time: {raw!r}")
    return days * 86400 + hours * 3600 + minutes * 60 + seconds


def parse_proc_stat_cpu_seconds(stat_line: str, clock_ticks: int) -> float:
    close = stat_line.rfind(")")
    if close == -1:
        raise ValueError("missing comm terminator in /proc stat line")
    fields = stat_line[close + 2 :].split()
    if len(fields) < 13:
        raise ValueError("short /proc stat line")
    return (int(fields[11]) + int(fields[12])) / float(clock_ticks)


def host_cpu_seconds_for_pid(pid: int) -> float:
    proc_stat = Path(f"/proc/{pid}/stat")
    if proc_stat.exists():
        clock_ticks = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
        return parse_proc_stat_cpu_seconds(proc_stat.read_text(encoding="utf-8"), clock_ticks)
    try:
        raw = subprocess.check_output(
            ["ps", "-o", "time=", "-p", str(pid)],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError as error:
        raise ProcessLookupError(pid) from error
    if not raw.strip():
        raise ProcessLookupError(pid)
    return parse_ps_time(raw)


def host_cpu_seconds(pids: list[int]) -> float:
    total = 0.0
    missing: list[int] = []
    for pid in pids:
        try:
            total += host_cpu_seconds_for_pid(pid)
        except (OSError, ValueError, ProcessLookupError):
            missing.append(pid)
    if missing:
        raise ProcessLookupError(f"process exited during idle sample: {missing}")
    return total


def adb_shell(adb: str, serial: str, command: str) -> str:
    argv = [adb]
    if serial:
        argv += ["-s", serial]
    argv += ["shell", command]
    return subprocess.check_output(argv, text=True, stderr=subprocess.STDOUT).replace("\r", "")


def android_pids(adb: str, serial: str, package: str) -> list[int]:
    raw = adb_shell(adb, serial, f"pidof {shlex.quote(package)} 2>/dev/null || true")
    pids: list[int] = []
    for token in raw.split():
        try:
            pids.append(int(token))
        except ValueError:
            pass
    if not pids:
        raise ProcessLookupError(f"no Android process found for {package}")
    return pids


def android_clock_ticks(adb: str, serial: str) -> int:
    raw = adb_shell(adb, serial, "getconf CLK_TCK 2>/dev/null || echo 100")
    try:
        ticks = int(raw.strip().splitlines()[-1])
    except (IndexError, ValueError) as error:
        raise RuntimeError(f"invalid Android CLK_TCK value: {raw!r}") from error
    if ticks <= 0:
        raise RuntimeError(f"invalid Android CLK_TCK value: {ticks}")
    return ticks


def android_cpu_seconds(adb: str, serial: str, pids: list[int], clock_ticks: int) -> float:
    raw = adb_shell(
        adb,
        serial,
        "for pid in " + " ".join(map(str, pids)) + "; do cat /proc/$pid/stat 2>/dev/null || exit 7; done",
    )
    total = 0.0
    lines = [line for line in raw.splitlines() if line.strip()]
    if len(lines) != len(pids):
        raise ProcessLookupError("Android process set changed during idle sample")
    for line in lines:
        total += parse_proc_stat_cpu_seconds(line, clock_ticks)
    return total


def sample_cpu_percent(
    read_cpu_seconds: Callable[[], float], sample_seconds: float, settle_seconds: float
) -> tuple[float, float]:
    if settle_seconds:
        time.sleep(settle_seconds)
    start_cpu = read_cpu_seconds()
    started = time.monotonic()
    time.sleep(sample_seconds)
    end_cpu = read_cpu_seconds()
    elapsed = time.monotonic() - started
    if elapsed <= 0:
        raise RuntimeError("idle CPU sample elapsed time was zero")
    return max(0.0, end_cpu - start_cpu) * 100.0 / elapsed, elapsed


def load_fixture(path: str | None) -> dict[str, Any] | None:
    if not path:
        return None
    with Path(path).open(encoding="utf-8") as handle:
        fixture = json.load(handle)
    errors: list[str] = []
    if fixture.get("loggedIn") is not True:
        errors.append("not logged in")
    if int(fixture.get("directChatCount", 0)) < 1:
        errors.append("no direct chat")
    if int(fixture.get("groupChatCount", 0)) < 1:
        errors.append("no group chat")
    if errors:
        raise ValueError("idle fixture is invalid: " + ", ".join(errors))
    return fixture


def write_result(path: str | None, result: dict[str, Any]) -> None:
    if not path:
        return
    output = Path(path)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def base_result(args: argparse.Namespace, mode: str) -> dict[str, Any]:
    return {
        "ok": False,
        "mode": mode,
        "label": args.label,
        "maxPercent": args.max_percent,
        "sampleSeconds": args.sample_seconds,
        "settleSeconds": args.settle_seconds,
        "generatedAt": generated_at(),
    }


def finish(result: dict[str, Any], artifact: str | None) -> int:
    write_result(artifact, result)
    label = result["label"]
    if result["ok"]:
        print(
            f"{label} idle CPU ok: {result['cpuPercent']:.3f}% <= "
            f"{result['maxPercent']:.3f}%"
        )
        if artifact:
            print(f"Result: {artifact}")
        return 0
    message = result.get("error")
    if not message:
        message = f"{result.get('cpuPercent', 0.0):.3f}% > {result['maxPercent']:.3f}%"
    print(f"{label} idle CPU gate failed: {message}", file=sys.stderr)
    if artifact:
        print(f"Result: {artifact}", file=sys.stderr)
    return 1


def run_sample(
    args: argparse.Namespace,
    mode: str,
    reader: Callable[[], float],
    metadata: dict[str, Any] | None = None,
) -> int:
    result = base_result(args, mode)
    if metadata:
        result.update(metadata)
    try:
        fixture = load_fixture(args.fixture)
        if fixture is not None:
            result["fixture"] = fixture
        cpu_percent, elapsed = sample_cpu_percent(reader, args.sample_seconds, args.settle_seconds)
        result["cpuPercent"] = cpu_percent
        result["elapsedSeconds"] = elapsed
        result["ok"] = cpu_percent <= args.max_percent
    except Exception as error:  # noqa: BLE001 - gate failures belong in the artifact.
        result["error"] = str(error)
    return finish(result, args.artifact)


def run_host_pid(args: argparse.Namespace) -> int:
    pids = [int(pid) for pid in args.pid]
    return run_sample(args, "host-pid", lambda: host_cpu_seconds(pids), {"processIds": pids})


def run_android_package(args: argparse.Namespace) -> int:
    try:
        pids = android_pids(args.adb, args.serial, args.package)
        clock_ticks = android_clock_ticks(args.adb, args.serial)
    except Exception as error:  # noqa: BLE001
        result = base_result(args, "android-package")
        result["error"] = str(error)
        return finish(result, args.artifact)
    return run_sample(
        args,
        "android-package",
        lambda: android_cpu_seconds(args.adb, args.serial, pids, clock_ticks),
        {
            "package": args.package,
            "serial": args.serial,
            "processIds": pids,
            "clockTicksPerSecond": clock_ticks,
        },
    )


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--label", default="Iris Chat")
    parser.add_argument("--artifact", help="JSON result path")
    parser.add_argument("--fixture", help="fixture proof JSON; must contain login, direct, and group state")
    parser.add_argument(
        "--max-percent",
        type=float,
        default=float(os.environ.get("IRIS_CHAT_IDLE_CPU_MAX_PERCENT", "5")),
    )
    parser.add_argument(
        "--sample-seconds",
        type=float,
        default=float(os.environ.get("IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS", "60")),
    )
    parser.add_argument(
        "--settle-seconds",
        type=float,
        default=float(os.environ.get("IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS", "30")),
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)
    host = subparsers.add_parser("host-pid")
    add_common(host)
    host.add_argument("--pid", action="append", required=True)
    host.set_defaults(func=run_host_pid)
    android = subparsers.add_parser("android-package")
    add_common(android)
    android.add_argument("--adb", default=os.environ.get("ADB", "adb"))
    android.add_argument("--serial", default=os.environ.get("ANDROID_SERIAL", ""))
    android.add_argument("--package", required=True)
    android.set_defaults(func=run_android_package)
    args = parser.parse_args()
    if args.max_percent < 0 or args.sample_seconds <= 0 or args.settle_seconds < 0:
        parser.error("CPU limits and durations must be non-negative; sample duration must be positive")
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
