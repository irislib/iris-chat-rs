#!/usr/bin/env python3
"""Compare a hosted disposable runner's release keychain without exposing it."""
import base64
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile


def security(*args, required=True):
    try:
        result = subprocess.run(["security", *args], capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        raise RuntimeError("Diagnostic keychain operation timed out") from None
    if required and result.returncode:
        raise RuntimeError("Diagnostic keychain operation failed")


if sys.argv[1] == "setup":
    root = Path(tempfile.mkdtemp(prefix="iris-notary-probe-", dir=os.environ["RUNNER_TEMP"]))
    keychain = root / "probe.keychain-db"
    with open(os.environ["GITHUB_ENV"], "a") as output:
        output.write("IRIS_NOTARY_PROBE_DIR=" + str(root) + "\n")
        output.write("IRIS_NOTARY_PROBE_KEYCHAIN=" + str(keychain) + "\n")
    password = secrets.token_hex(24)
    certificate = root / "certificate.p12"
    fd = os.open(certificate, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(base64.b64decode(os.environ["MACOS_CERTIFICATE_P12_BASE64"]))
    try:
        security("create-keychain", "-p", password, str(keychain))
        security("set-keychain-settings", "-lut", "21600", str(keychain))
        security("unlock-keychain", "-p", password, str(keychain))
        security("import", str(certificate), "-P", os.environ["MACOS_CERTIFICATE_PASSWORD"], "-A", "-t", "cert", "-f", "pkcs12", "-k", str(keychain))
        security("set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", password, str(keychain))
        security("list-keychains", "-d", "user", "-s", str(keychain), "login.keychain-db")
        security("default-keychain", "-s", str(keychain))
    finally:
        certificate.unlink(missing_ok=True)
    print("Temporary release signing keychain configured; no artifact was signed or changed.")
elif sys.argv[1] == "cleanup":
    raw = os.environ.get("IRIS_NOTARY_PROBE_DIR")
    if raw:
        root = Path(raw)
        assert root.parent == Path(os.environ["RUNNER_TEMP"]) and root.name.startswith("iris-notary-probe-")
        assert not root.is_symlink()
        security("default-keychain", "-s", "login.keychain-db")
        security("list-keychains", "-d", "user", "-s", "login.keychain-db")
        security("delete-keychain", str(root / "probe.keychain-db"), required=False)
        shutil.rmtree(root)
    print("Temporary diagnostic signing material removed.")
else:
    raise SystemExit("Expected setup or cleanup")
