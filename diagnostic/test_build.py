import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import build


class InputTests(unittest.TestCase):
    def test_payload_hashes_match(self):
        self.assertTrue(build.verify_inputs()["diagnostic_only"])

    def test_only_selected_registry_locations_may_change(self):
        a = {"package": [{"name": "a", "version": "1", "source": "registry", "checksum": "x", "dependencies": ["b"]}]}
        b = {"package": [{"name": "a", "version": "1", "dependencies": ["b"]}]}
        self.assertEqual(build.lock_shape(a, {("a", "1")}), build.lock_shape(b, {("a", "1")}))
        self.assertNotEqual(build.lock_shape(a, set()), build.lock_shape(b, set()))
        b["package"][0]["dependencies"] = ["c"]
        self.assertNotEqual(build.lock_shape(a, {("a", "1")}), build.lock_shape(b, {("a", "1")}))

    def test_archive_cannot_escape_or_link(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name, kind in [("../escape", tarfile.REGTYPE), ("pkg/link", tarfile.SYMTYPE)]:
                archive = root / "bad.crate"
                with tarfile.open(archive, "w") as tar:
                    entry = tarfile.TarInfo(name)
                    entry.type = kind
                    entry.linkname = "/outside"
                    tar.addfile(entry)
                with self.assertRaises(ValueError):
                    build.unpack_crate(archive, root / "out", "pkg")

    def test_regular_crate_extracts(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / "ok.crate"
            with tarfile.open(archive, "w") as tar:
                entry = tarfile.TarInfo("pkg/src/lib.rs")
                entry.size = 2
                tar.addfile(entry, io.BytesIO(b"ok"))
            build.unpack_crate(archive, root / "out", "pkg")
            self.assertEqual((root / "out/pkg/src/lib.rs").read_bytes(), b"ok")

    def test_disk_floor_refuses_before_launch(self):
        with patch.object(build.shutil, "disk_usage", return_value=SimpleNamespace(free=0)):
            with patch.object(build.subprocess, "Popen") as launch:
                with self.assertRaisesRegex(RuntimeError, "Disk floor"):
                    build.bounded_build(["unused"], Path("."), {}, Path("unused.log"))
                launch.assert_not_called()

    def test_existing_work_directory_is_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(build, "run") as execute:
                with self.assertRaises(FileExistsError):
                    build.prepare("client", Path(folder), build.verify_inputs())
                execute.assert_not_called()

    def test_platform_binding_prevents_wrong_provider(self):
        build.check_platform("client", "Linux", "x86_64")
        build.check_platform("provider", "Darwin", "arm64")
        with self.assertRaises(RuntimeError):
            build.check_platform("provider", "Linux", "x86_64")
        with self.assertRaises(RuntimeError):
            build.check_platform("provider", "Darwin", "x86_64")


if __name__ == "__main__":
    unittest.main()
