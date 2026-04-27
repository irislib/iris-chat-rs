#!/usr/bin/env python3
import os
import subprocess
import sys
from pathlib import Path


def main() -> int:
    root_dir = Path(__file__).resolve().parent.parent
    bind_addr = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("IRIS_LOCAL_RELAY_BIND", "0.0.0.0:4848")
    command = [
        "cargo",
        "run",
        "--manifest-path",
        str(root_dir / "core" / "Cargo.toml"),
        "--bin",
        "local_nostr_relay",
        "--",
        bind_addr,
    ]
    completed = subprocess.run(command)
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
