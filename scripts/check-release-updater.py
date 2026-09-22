#!/usr/bin/env python3
"""Check public signed discovery with the exact, attested release executable."""

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path


def verify(cli: Path, tag: str) -> None:
    environment = {
        name: value for name, value in os.environ.items()
        if not name.startswith("IRIS_UPDATE_")
    }
    with tempfile.TemporaryDirectory(prefix="iris-release-update-") as data_dir:
        for app in (False, True):
            arguments = [str(cli.resolve()), "--data-dir", data_dir,
                         "update", "check", "--source", "hashtree", "--json"]
            if app:
                arguments.append("--app")
            result = subprocess.run(arguments, env=environment, capture_output=True,
                                    text=True, timeout=60)
            if result.returncode:
                raise ValueError(f"signed updater check failed: {result.stdout}{result.stderr}")
            update = json.loads(result.stdout)
            prefix = "iris-chat-" if app else "iris-"
            if (update.get("tag") != tag or update.get("verified") is not True
                    or update.get("source") != "hashtree-nostr-blossom"
                    or not update.get("asset", "").startswith(f"{prefix}{tag}-")):
                raise ValueError(f"signed updater returned a different or unverified release: {update}")
            print(f"Verified signed {'app' if app else 'CLI'} update: {update['asset']}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    try:
        verify(args.cli, args.tag)
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"{error}\n")
