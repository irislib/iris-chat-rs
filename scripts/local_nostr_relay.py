#!/usr/bin/env python3
"""Build the local Rust relay binary then exec it directly.

Using `cargo run` would leave us as a parent of the rust binary, and
killing this python wouldn't reliably kill the relay (since the rust
binary inherits the process group but cargo's reaping isn't guaranteed
on a SIGTERM). Build with `cargo build`, then `os.execv` the binary so
the OS replaces this python process with the rust process — the PID
the harness tracks IS the relay, and a single kill terminates it.
"""
import json
import os
import subprocess
import sys
from pathlib import Path


def cargo_target_dir(core_dir: Path) -> Path:
    metadata = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(core_dir / "Cargo.toml"),
            "--format-version",
            "1",
            "--no-deps",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(json.loads(metadata.stdout)["target_directory"])


def main() -> int:
    root_dir = Path(__file__).resolve().parent.parent
    core_dir = root_dir / "core"
    bind_addr = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("IRIS_LOCAL_RELAY_BIND", "0.0.0.0:4848")
    manifest = str(core_dir / "Cargo.toml")
    build = subprocess.run([
        "cargo", "build",
        "--manifest-path", manifest,
        "--features", "local-relay-bin",
        "--bin", "local_nostr_relay",
    ])
    if build.returncode != 0:
        return build.returncode
    binary = cargo_target_dir(core_dir) / "debug" / "local_nostr_relay"
    if not binary.exists():
        sys.stderr.write(f"local_nostr_relay binary not found at {binary}\n")
        return 1
    os.execv(str(binary), [str(binary), bind_addr])


if __name__ == "__main__":
    raise SystemExit(main())
