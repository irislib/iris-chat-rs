#!/usr/bin/env python3
"""Check public signed discovery with the exact, attested release executable."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import subprocess
import tempfile
import uuid
from pathlib import Path


def linux_ca_file(configured: Path | None) -> Path:
    candidates = [configured] if configured else [
        Path("/etc/ssl/certs/ca-certificates.crt"), Path("/etc/ssl/cert.pem")]
    for path in candidates:
        if path.is_file():
            return path.resolve()
    raise ValueError("isolated check needs a CA bundle; pass --ca-file")


def isolated_check(cli: Path, ca_file: Path, app: bool, environment: dict) -> subprocess.CompletedProcess:
    name = "iris-release-update-" + uuid.uuid4().hex
    arguments = [
        "docker", "run", "--rm", "--name", name, "--read-only",
        "--platform", "linux/amd64", "--network", "bridge",
        "--tmpfs", "/tmp", "--tmpfs", "/profile",
        "--mount", f"type=bind,source={cli},target=/release/iris,readonly",
        "--mount", f"type=bind,source={ca_file},target=/release/ca.pem,readonly",
        "--env", "SSL_CERT_FILE=/release/ca.pem", "--entrypoint", "/release/iris",
        "ubuntu:24.04", "--data-dir", "/profile",
        "update", "check", "--source", "hashtree", "--json",
    ]
    if app:
        arguments.append("--app")
    try:
        return subprocess.run(arguments, env=environment, capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        # Killing the Docker client does not stop its container. Remove only
        # this check's randomly named container before reporting the timeout.
        subprocess.run(["docker", "rm", "--force", name], env=environment,
                       capture_output=True, text=True, timeout=10)
        raise


def verify(cli: Path, tag: str, *, isolated_linux: bool = False,
           ca_file: Path | None = None, receipt: Path | None = None) -> None:
    cli = cli.resolve()
    environment = {
        name: value for name, value in os.environ.items()
        if not name.startswith(("IRIS_UPDATE_", "IRIS_FIPS_"))
    }
    if isolated_linux:
        with cli.open("rb") as executable:
            header = executable.read(20)
        if header[:6] != b"\x7fELF\x02\x01" or header[18:20] != b"\x3e\x00":
            raise ValueError("isolated check requires the attested Linux x64 CLI executable")
        ca_file = linux_ca_file(ca_file)
    checks = []
    for app in (False, True):
        if isolated_linux:
            result = isolated_check(cli, ca_file, app, environment)
        else:
            with tempfile.TemporaryDirectory(prefix="iris-release-update-") as data_dir:
                arguments = [str(cli), "--data-dir", data_dir,
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
        checks.append({"mode": "app" if app else "cli", "update": update})
        print(f"Verified signed {'app' if app else 'CLI'} update: {update['asset']}")
    if receipt:
        receipt.parent.mkdir(parents=True, exist_ok=True)
        receipt.write_text(json.dumps({
            "checked_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "tag": tag, "cli_sha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
            "isolation": "docker-linux-amd64-bridge" if isolated_linux else "native-data-directory",
            "checks": checks,
        }, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--isolated-linux", action="store_true",
                        help="require Linux x64 CLI; check each mode in a fresh Docker bridge namespace")
    parser.add_argument("--ca-file", type=Path, help="host CA bundle for the isolated Linux container")
    parser.add_argument("--receipt", type=Path, help="write JSON evidence after both checks pass")
    args = parser.parse_args()
    try:
        verify(args.cli, args.tag, isolated_linux=args.isolated_linux,
               ca_file=args.ca_file, receipt=args.receipt)
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"{error}\n")
