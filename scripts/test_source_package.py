#!/usr/bin/env python3
"""Offline tamper checks for the source-package archive acceptance boundary."""
import importlib.util
import io
import json
import os
import subprocess
import sys
import time
from types import SimpleNamespace
from unittest import mock
from pathlib import Path
import tarfile
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("package_check", Path(__file__).with_name("verify_source_package.py"))
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class PackageProofTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.archive = Path(self.temporary.name) / "iris-chat-0.1.49.crate"
        manifest = b'''[package]
name = "iris-chat"
version = "0.1.49"
default-run = "iris"
[dependencies]
iris-chat-protocol = { version = "=0.3.7", path = "../chat-protocol" }
[[bin]]
name = "iris"
path = "src/bin/iris.rs"
'''
        lock = b'''version = 4
[[package]]
name = "iris-chat"
version = "0.1.49"
dependencies = ["iris-chat-protocol"]
[[package]]
name = "iris-chat-protocol"
version = "0.3.7"
'''
        self.checksum = "a" * 64
        self.source = {"core/Cargo.toml": manifest, "core/Cargo.lock": lock}
        for path in ["README.md", "build.rs", "src/bin/iris.rs", "assets/emoji-shortcodes.tsv", "assets/socialGraph.bin"]:
            self.source[f"core/{path}"] = path.encode()
        self.members = {path.removeprefix("core/"): content for path, content in self.source.items()}
        self.members["Cargo.toml.orig"] = manifest
        self.members["Cargo.toml"] = manifest.replace(b', path = "../chat-protocol"', b'')
        self.members["Cargo.lock"] = lock + f'source = "{CHECK.REGISTRY}"\nchecksum = "{self.checksum}"\n'.encode()
        self.members[".cargo_vcs_info.json"] = json.dumps({"git": {"sha1": CHECK.SOURCE, "dirty": False}, "path_in_vcs": "core"}).encode()

    def inspect(self, extra=()):
        with tarfile.open(self.archive, "w:gz") as tar:
            for path, content in [*self.members.items(), *extra]:
                item = tarfile.TarInfo(f"iris-chat-0.1.49/{path}")
                item.size = len(content)
                tar.addfile(item, io.BytesIO(content))
        return CHECK.inspect_archive(self.archive, "iris-chat", "0.1.49", "core",
                                     self.source.__getitem__, self.checksum, expected_source=CHECK.SOURCE)

    def inspect_protocol(self, archive_source, expected_source):
        manifest = b'[package]\nname = "iris-chat-protocol"\nversion = "0.3.7"\n'
        lock = b'version = 4\n[[package]]\nname = "iris-chat-protocol"\nversion = "0.3.7"\n'
        source = {"chat-protocol/Cargo.toml": manifest, "chat-protocol/Cargo.lock": lock,
                  "chat-protocol/src/lib.rs": b"pub struct Protocol;\n"}
        members = {path.removeprefix("chat-protocol/"): content for path, content in source.items()}
        members["Cargo.toml.orig"] = manifest
        members[".cargo_vcs_info.json"] = json.dumps({
            "git": {"sha1": archive_source, "dirty": False}, "path_in_vcs": "chat-protocol"}).encode()
        archive = Path(self.temporary.name) / "iris-chat-protocol-0.3.7.crate"
        with tarfile.open(archive, "w:gz") as tar:
            for path, content in members.items():
                item = tarfile.TarInfo(f"iris-chat-protocol-0.3.7/{path}")
                item.size = len(content)
                tar.addfile(item, io.BytesIO(content))
        return CHECK.inspect_archive(archive, "iris-chat-protocol", "0.3.7", "chat-protocol",
                                     source.__getitem__, expected_source=expected_source)

    def test_accepts_different_cli_and_unchanged_protocol_origins(self):
        with mock.patch.object(CHECK, "SOURCE", "c" * 40):
            self.members[".cargo_vcs_info.json"] = json.dumps({
                "git": {"sha1": CHECK.SOURCE, "dirty": False}, "path_in_vcs": "core"}).encode()
            cli = self.inspect()
            protocol = self.inspect_protocol(CHECK.PROTOCOL_SOURCE, CHECK.PROTOCOL_SOURCE)
        self.assertEqual(cli["vcs"]["git"]["sha1"], "c" * 40)
        self.assertEqual(protocol["vcs"]["git"]["sha1"], CHECK.PROTOCOL_SOURCE)
        self.assertTrue(cli["all_source_bytes_match_git"] and protocol["all_source_bytes_match_git"])

    def test_rejects_swapped_or_wrong_package_origins(self):
        with mock.patch.object(CHECK, "SOURCE", "c" * 40):
            with self.assertRaisesRegex(AssertionError, "Archive source commit"):
                self.inspect()
            for actual, expected in [(CHECK.PROTOCOL_SOURCE, CHECK.SOURCE),
                                     (CHECK.SOURCE, CHECK.PROTOCOL_SOURCE),
                                     ("b" * 40, CHECK.PROTOCOL_SOURCE)]:
                with self.subTest(actual=actual, expected=expected), \
                     self.assertRaisesRegex(AssertionError, "Archive source commit"):
                    self.inspect_protocol(actual, expected)

    def test_accepts_exact_source_and_registry_normalization(self):
        result = self.inspect()
        self.assertTrue(result["all_source_bytes_match_git"])
        self.assertEqual(result["dependency_count"], 2)

    def test_rejects_changed_source_bytes(self):
        self.members["src/bin/iris.rs"] += b"tampered"
        with self.assertRaisesRegex(AssertionError, "Source bytes differ"):
            self.inspect()

    def test_rejects_wrong_source_commit_and_dirty_source(self):
        for vcs in [{"git": {"sha1": "b" * 40}, "path_in_vcs": "core"},
                    {"git": {"sha1": CHECK.SOURCE, "dirty": True}, "path_in_vcs": "core"}]:
            with self.subTest(vcs=vcs), self.assertRaises(AssertionError):
                self.members[".cargo_vcs_info.json"] = json.dumps(vcs).encode()
                self.inspect()

    def test_rejects_relaxed_protocol_pin(self):
        self.members["Cargo.toml"] = self.members["Cargo.toml"].replace(b"=0.3.7", b"0.3.7")
        with self.assertRaises(AssertionError):
            self.inspect()

    def test_rejects_changed_registry_checksum(self):
        self.members["Cargo.lock"] = self.members["Cargo.lock"].replace(self.checksum.encode(), b"b" * 64)
        with self.assertRaisesRegex(AssertionError, "lock dependency change"):
            self.inspect()

    def test_rejects_changed_dependency_version(self):
        self.members["Cargo.lock"] = self.members["Cargo.lock"].replace(b"0.3.7", b"0.3.8")
        with self.assertRaisesRegex(AssertionError, "lock dependency change"):
            self.inspect()

    def test_rejects_traversal_and_duplicate_files(self):
        for path in ["../outside", "README.md"]:
            with self.subTest(path=path), self.assertRaises(AssertionError):
                self.inspect([(path, b"bad")])

    def test_rejects_missing_required_package_asset(self):
        del self.members["assets/socialGraph.bin"]
        with self.assertRaisesRegex(AssertionError, "Required package file missing"):
            self.inspect()


class PackageProcessCleanupTests(unittest.TestCase):
    def run_injected_failure(self, kind):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out, target = root / "evidence", root / "target"
            out.mkdir(); target.mkdir()
            candidate = target / "package" / "iris-chat-0.1.49.crate"
            real_popen = subprocess.Popen
            unrelated = real_popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                   start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            children = []
            try:
                def synthetic_child(command, **kwargs):
                    self.assertEqual(command[:3], ["cargo", "+1.99.1", "package"])
                    self.assertTrue(kwargs["start_new_session"])
                    child = real_popen([sys.executable, "-u", "-c",
                        "import pathlib,sys,time; p=pathlib.Path(sys.argv[1]); p.parent.mkdir(); "
                        "p.write_bytes(b'synthetic archive'); print('synthetic package started',flush=True); time.sleep(30)",
                        str(candidate)], **kwargs)
                    children.append(child)
                    deadline = time.monotonic() + 2
                    while not candidate.exists() or not (out / "cargo-package.log").stat().st_size:
                        self.assertIsNone(child.poll())
                        self.assertLess(time.monotonic(), deadline)
                        time.sleep(0.01)
                    return child
                disk = SimpleNamespace(free=6 * CHECK.GIB)
                disk_effect = [disk, OSError("injected disk probe failure")] if kind == "disk" else None
                rss_effect = subprocess.TimeoutExpired(["ps"], 10) if kind == "monitor" else None
                with mock.patch.object(CHECK.subprocess, "Popen", side_effect=synthetic_child), \
                     mock.patch.object(CHECK.shutil, "disk_usage", return_value=disk, side_effect=disk_effect), \
                     mock.patch.object(CHECK, "memory_total", return_value=16 * CHECK.GIB), \
                     mock.patch.object(CHECK, "group_rss", return_value=0, side_effect=rss_effect):
                    with self.assertRaises((RuntimeError, AssertionError)):
                        CHECK.bounded_package(root, out, target, "1.99.1",
                                              timeout_seconds=0.001 if kind == "timeout" else 1,
                                              poll_seconds=0.01)
                self.assertEqual(len(children), 1)
                self.assertIsNotNone(children[0].returncode)
                with self.assertRaises(ProcessLookupError):
                    os.kill(children[0].pid, 0)
                self.assertIsNone(unrelated.poll(), "Cleanup touched an unrelated process")
                status = json.loads((out / "package-status.json").read_text())
                self.assertTrue(status["owned_process_reaped"])
                self.assertIsNone(status["cleanup_error"])
                expected = "Package time limit reached" if kind == "timeout" else "resource monitor failed:"
                self.assertIn(expected, status["stop_reason"])
                self.assertIn("synthetic package started", (out / "cargo-package.log").read_text())
                self.assertEqual((out / candidate.name).read_bytes(), b"synthetic archive")
            finally:
                for child in [*children, unrelated]:
                    if child.poll() is None:
                        child.terminate()
                    child.wait(timeout=3)

    def test_ps_monitor_exception_stops_and_reaps_owned_child(self):
        self.run_injected_failure("monitor")

    def test_disk_probe_exception_stops_and_reaps_owned_child(self):
        self.run_injected_failure("disk")

    def test_timeout_stops_and_reaps_owned_child(self):
        self.run_injected_failure("timeout")


if __name__ == "__main__":
    unittest.main()
