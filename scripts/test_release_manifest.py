#!/usr/bin/env python3

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "scripts" / "release-manifest.py"
TAG = "v2026.7.28"


def names(tag: str) -> list[str]:
    return [
        f"iris-chat-{tag}-android-arm64.apk",
        f"iris-chat-{tag}-android-arm64.aab",
        f"iris-chat-{tag}-ios.ipa",
        f"iris-chat-{tag}-ios.xcarchive.zip",
        f"iris-chat-{tag}-macos-arm64.dmg",
        f"iris-chat-{tag}-macos-arm64.app.tar.gz",
        f"iris-chat-{tag}-windows-x64-setup.exe",
        f"iris-chat-{tag}-windows-x64.zip",
        f"iris-chat-{tag}-linux-x64.deb",
        f"iris-chat-{tag}-linux-x64.tar.gz",
        f"iris-{tag}-aarch64-apple-darwin.tar.gz",
        f"iris-{tag}-x86_64-apple-darwin.tar.gz",
        f"iris-{tag}-x86_64-unknown-linux-gnu.tar.gz",
    ]


class ReleaseManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        for name in names(TAG):
            (self.assets / name).write_bytes(name.encode())
        self.manifest = self.root / f"iris-chat-{TAG}-manifest.json"

    def tearDown(self) -> None:
        self.directory.cleanup()

    def create(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                str(MANIFEST),
                "create",
                "--tag",
                TAG,
                "--commit",
                "abc123",
                "--asset-dir",
                str(self.assets),
                "--out",
                str(self.manifest),
            ],
            capture_output=True,
            text=True,
        )

    def test_create_and_verify_exact_manifest(self) -> None:
        result = self.create()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(self.manifest.read_text())
        self.assertEqual(data["tag"], TAG)
        self.assertEqual(len(data["assets"]), 13)
        self.assertTrue(all(asset["sha256"] for asset in data["assets"]))

        result = subprocess.run(
            [
                str(MANIFEST),
                "verify",
                "--tag",
                TAG,
                "--manifest",
                str(self.manifest),
                "--asset-dir",
                str(self.assets),
                "--require-name",
                names(TAG)[0],
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_rolling_or_unexpected_asset(self) -> None:
        (self.assets / "IrisChat-release-latest.apk").write_bytes(b"bad")
        result = self.create()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected", result.stderr)

    def test_rejects_non_calendar_release_tag(self) -> None:
        for tag in ("v2026.2.30", "v2026.7.28.0", "v2026.07.28"):
            with self.subTest(tag=tag):
                result = subprocess.run(
                    [str(MANIFEST), "validate-tag", "--tag", tag],
                    capture_output=True,
                    text=True,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsupported stable release tag", result.stderr)

    def test_rejects_modified_download(self) -> None:
        self.assertEqual(self.create().returncode, 0)
        name = names(TAG)[0]
        (self.assets / name).write_bytes(b"modified")
        result = subprocess.run(
            [
                str(MANIFEST),
                "verify",
                "--tag",
                TAG,
                "--manifest",
                str(self.manifest),
                "--asset-dir",
                str(self.assets),
                "--require-name",
                name,
            ],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mismatch", result.stderr)

    def test_rejects_manifest_for_another_commit(self) -> None:
        self.assertEqual(self.create().returncode, 0)
        result = subprocess.run(
            [
                str(MANIFEST),
                "verify",
                "--tag",
                TAG,
                "--commit",
                "different",
                "--manifest",
                str(self.manifest),
                "--asset-dir",
                str(self.assets),
                "--require-name",
                names(TAG)[0],
            ],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("commit mismatch", result.stderr)

    def test_hashtree_stage_preserves_manifest_and_asset_names(self) -> None:
        self.assertEqual(self.create().returncode, 0)
        notes = self.root / "notes.md"
        notes.write_text("- Change.\n")
        stage = self.root / "stage"
        result = subprocess.run(
            [
                str(MANIFEST),
                "stage-hashtree",
                "--tag",
                TAG,
                "--manifest",
                str(self.manifest),
                "--asset-dir",
                str(self.assets),
                "--notes",
                str(notes),
                "--out-dir",
                str(stage),
                "--published-at",
                "2026-07-28T10:00:00Z",
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest_name = self.manifest.name
        self.assertEqual(
            (stage / "assets" / manifest_name).read_bytes(),
            self.manifest.read_bytes(),
        )
        release = json.loads((stage / "release.json").read_text())
        staged_names = {entry["name"] for entry in release["assets"]}
        self.assertEqual(staged_names, {*names(TAG), manifest_name})


if __name__ == "__main__":
    unittest.main()
