import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from android_saved_fixture import SavedFixture, normal_fixture_environment, private_file, validate_pair


class SavedFixtureTest(unittest.TestCase):
    def test_normal_networking_rejects_control_or_ambient_policy_overrides(self):
        source = {"PATH": "example", "IRIS_DEMO_RELAYS": "previous"}
        self.assertEqual({"PATH": "example", "IRIS_DEMO_RELAYS": "ws://127.0.0.1:1234"},
                         normal_fixture_environment(source, "ws://127.0.0.1:1234"))
        self.assertEqual("previous", source["IRIS_DEMO_RELAYS"])
        for key in ("IRIS_CALL_ISOLATED_CONTROL", "IRIS_FIPS_WEBSOCKET_SEED_URLS", "IRIS_CHAT_FIPS_STATIC_PEERS",
                    "IRIS_CALL_PUSH_SERVER_URL", "IRIS_UPDATE_RELAYS", "IRIS_RUNTIME_DEBUG_SNAPSHOT"):
            with self.assertRaises(AssertionError): normal_fixture_environment({key: ""}, "ws://127.0.0.1:1234")
        for relay in ("wss://public.example", "ws://127.0.0.1:0", "ws://127.0.0.1:65536", "ws://localhost:1234"):
            with self.assertRaises(AssertionError): normal_fixture_environment({}, relay)

    def test_persistent_pair_requires_both_receiver_keys_and_unchanged_private_receipt(self):
        value = dict(version=1, mode="normal", receiver=dict(owner="a"*64, device="b"*64),
            relay="ws://127.0.0.1:1234", sender=dict(owner="c"*64, device="d"*64, device_npub="npub1example"),
            secret_receipt_sha256="e"*64)
        self.assertEqual(value["sender"], validate_pair(value, "a"*64, "b"*64, value["relay"], "e"*64))
        for mutate in (lambda v: v["receiver"].update(owner="f"*64), lambda v: v["receiver"].update(device="f"*64),
                       lambda v: v.update(relay="ws://127.0.0.1:4321"), lambda v: v.update(mode="control"),
                       lambda v: v.update(secret_receipt_sha256="f"*64)):
            changed = copy.deepcopy(value); mutate(changed)
            with self.assertRaises(AssertionError): validate_pair(changed, "a"*64, "b"*64, value["relay"], "e"*64)

    def test_fixture_reuses_one_private_identity_and_refuses_partial_or_modified_receipts(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            root = Path(directory); fixture_dir = root / "fixture"; calls = []
            ready = dict(event="ready", owner="c"*64, device="d"*64, device_npub="npub1example", invite="fixture-invite")
            class Process:
                def __init__(self):
                    self.stdin = io.StringIO(); self.stdout = io.StringIO(json.dumps(ready) + '\n{"event":"accepted"}\n')
                    self.finished = False
                def poll(self): return 0 if self.finished else None
                def terminate(self): self.finished = True
                def wait(self, timeout): return 0
            def spawn(arguments, **kwargs):
                calls.append(arguments)
                secret = fixture_dir / "fixture-account-bundle.json"
                if not secret.exists():
                    with os.fdopen(os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as file:
                        file.write("private synthetic key receipt")
                return Process()
            def fixture(): return SavedFixture(Path("fixture-bin"), fixture_dir, root, "a"*64, "b"*64, "ws://127.0.0.1:1234")
            with patch("android_saved_fixture.subprocess.Popen", side_effect=spawn):
                first = fixture()
                try: self.assertEqual(ready, first.start())
                finally: first.stop()
                self.assertEqual("--persist-normal", calls[0][-1])
                receipt = (fixture_dir / "fixture-account-bundle.json").read_bytes()
                pair = private_file(fixture_dir / "receiver-pair.json")
                second = fixture()
                try: self.assertEqual(ready, second.start())
                finally: second.stop()
                self.assertEqual(["--resume-normal", ready["owner"], ready["device_npub"]], calls[1][-3:])
                self.assertEqual(pair, private_file(fixture_dir / "receiver-pair.json"))
                self.assertEqual(hashlib.sha256(receipt).hexdigest(), json.loads(pair)["secret_receipt_sha256"])
                (fixture_dir / "fixture-account-bundle.json").write_text("changed")
                with self.assertRaises(AssertionError): fixture().start()
                self.assertEqual(2, len(calls))
                (fixture_dir / "receiver-pair.json").unlink()  # Test-owned temporary data only.
                with self.assertRaises(FileNotFoundError): fixture().start()
                self.assertEqual(2, len(calls))

    def test_idle_cleanup_attempts_both_owned_restorations(self):
        fixture = SavedFixture(None, None, None, None, None, None)
        fixture.forced_idle = True; calls = []
        def adb(*args):
            calls.append(args)
            if args[-1] == "unforce": raise RuntimeError("simulated failure")
            return ""
        with self.assertRaises(AssertionError): fixture.restore_device(adb)
        self.assertEqual(["unforce", "reset"], [parts[-1] for parts in calls])
        self.assertTrue(fixture.forced_idle)


if __name__ == "__main__": unittest.main()
