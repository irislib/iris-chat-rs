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

    completed = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    sys.stdout.write(completed.stdout.decode("utf-8", errors="replace"))
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
