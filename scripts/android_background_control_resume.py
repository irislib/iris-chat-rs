"""Fail-closed continuation of the exact failed, paired control2 installation."""
import hashlib
import json
import re
from pathlib import Path

from android_background_health import CONTROL_PACKAGE


def installed_artifact_hashes(adb, app_apk, test_apk):
    result = {}
    for package, artifact in [(CONTROL_PACKAGE, app_apk), (CONTROL_PACKAGE + ".test", test_apk)]:
        paths = adb("shell", "pm", "path", package).strip().splitlines()
        assert len(paths) == 1 and paths[0].startswith("package:"), "Expected one installed control artifact"
        path = paths[0][len("package:"):]
        assert re.fullmatch(r"/data/app/[-A-Za-z0-9_./=+~]+/base\.apk", path), "Unexpected installed APK location"
        digest = adb("shell", "sha256sum", path).split()[0]
        assert re.fullmatch(r"[a-f0-9]{64}", digest)
        assert digest == hashlib.sha256(Path(artifact).read_bytes()).hexdigest(), "Installed control artifact changed"
        result[package] = digest
    return result


def paired_resume_settings(adb, source, host):
    source = Path(source).resolve()
    assert not (source / "profile-ready.json").exists(), "Only a setup failure before measurement can continue"
    assert json.loads((source / "cleanup.json").read_text()) == {"errors": []}
    for name, count in [("parser-tests.log", 10), ("bootstrap.log", 1), ("pairing.log", 1)]:
        assert f"OK ({count} test{'s' if count != 1 else ''})" in (source / name).read_text()
    assert f"{CONTROL_PACKAGE}/to.iris.chat.MainActivity" in (source / "normal-launch.log").read_text()
    assert not adb("shell", "pidof", CONTROL_PACKAGE, check=False).strip(), "Control must be stopped before resume"
    config = json.loads(adb("shell", "run-as", CONTROL_PACKAGE, "cat", "cache/background-control.json"))
    assert set(config) == {"phase", "relay_url", "peer_npub", "peer_udp", "local_udp_port"}
    assert config["phase"] == "paired"
    relay = re.fullmatch(r"ws://127\.0\.0\.1:([0-9]+)", config["relay_url"])
    assert relay and 0 < int(relay[1]) <= 65535
    peer = re.fullmatch(re.escape(host) + r":([0-9]+)", config["peer_udp"])
    assert peer and 0 < int(peer[1]) <= 65535, "Preserved static host address changed"
    assert re.fullmatch(r"npub1[023456789acdefghjklmnpqrstuvwxyz]{58}", config["peer_npub"])
    assert type(config["local_udp_port"]) is int and 1024 <= config["local_udp_port"] <= 65535
    receipt = source / "host-account" / "fixture-account-bundle.json"
    assert not receipt.is_symlink() and receipt.is_file() and receipt.stat().st_mode & 0o777 == 0o600
    assert 0 < receipt.stat().st_size <= 4096 and (receipt.parent / "core.sqlite3").is_file()
    # Return only public identity; native restore independently validates both secret keys
    # and derives the exact paired device before constructing the core or networking.
    bundle = json.loads(receipt.read_text())
    assert set(bundle) == {"version", "owner_nsec", "owner_pubkey_hex", "device_nsec"}
    assert bundle["version"] == 1 and re.fullmatch(r"[a-f0-9]{64}", bundle["owner_pubkey_hex"])
    bootstrap = (source / "bootstrap.log").read_text()
    def field(name, pattern):
        match = re.search(r"INSTRUMENTATION_STATUS: " + name + "=(" + pattern + ")", bootstrap)
        assert match is not None, "Missing preserved phone identity"
        return match[1]
    assert int(field("udp_port", r"[0-9]+")) == config["local_udp_port"]
    return {"source": source, "config": config, "relay_port": int(relay[1]), "host_port": int(peer[1]),
            "owner": field("owner", r"[a-f0-9]{64}"),
            "device_npub": field("device_npub", r"npub1[023456789acdefghjklmnpqrstuvwxyz]{58}"),
            "host_owner": bundle["owner_pubkey_hex"], "host_account": receipt.parent}
