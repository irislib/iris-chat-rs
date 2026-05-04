#!/usr/bin/env python3

import argparse
import base64
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description="Run an Android instrumentation harness test with quote-safe arguments.")
    parser.add_argument("--adb", required=True, help="Absolute path to adb")
    parser.add_argument("--serial", default="", help="adb device serial. Omit when adb has a single target device.")
    parser.add_argument("--runner", required=True, help="Instrumentation runner package/class")
    parser.add_argument("--class-name", required=True, help="Harness test class, without #method")
    parser.add_argument("--test-name", required=True, help="Harness test method")
    parser.add_argument("--user", default="0", help="Android user id")
    parser.add_argument(
        "--arg",
        action="append",
        default=[],
        help="Instrumentation argument in KEY=VALUE form. Values are base64-encoded before dispatch.",
    )
    args = parser.parse_args()

    command = [
        args.adb,
        "shell",
        "am",
        "instrument",
        "-w",
        "-r",
        "--user",
        args.user,
    ]
    if args.serial:
        command[1:1] = ["-s", args.serial]
    for item in args.arg:
        if "=" not in item:
            raise SystemExit(f"Invalid --arg `{item}`. Expected KEY=VALUE.")
        key, value = item.split("=", 1)
        encoded = base64.urlsafe_b64encode(value.encode("utf-8")).decode("ascii")
        command.extend(["-e", f"{key}_b64", encoded])

    command.extend(["-e", "class", f"{args.class_name}#{args.test_name}", args.runner])

    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    assert process.stdout is not None
    saw_failure = False
    saw_success_code = False
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        stripped = line.strip()
        if stripped == "INSTRUMENTATION_CODE: -1":
            saw_success_code = True
        if (
            stripped.startswith("INSTRUMENTATION_STATUS_CODE: -")
            or stripped == "FAILURES!!!"
            or stripped.startswith("INSTRUMENTATION_RESULT: shortMsg=")
        ):
            saw_failure = True

    exit_code = process.wait()
    if exit_code != 0:
        return exit_code
    if saw_failure or not saw_success_code:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
