#!/usr/bin/env python3

import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "scripts" / "release-manifest.py"
TAG = "v2026.7.28"
IOS_EXCLUDED_TAG = "v2026.10.8.2"


def names(tag: str, *, include_ios: bool = True) -> list[str]:
    assets = [
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
    return assets if include_ios else [name for name in assets if "-ios." not in name]


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

    def test_default_tags_still_require_both_ios_assets(self) -> None:
        for name in names(TAG):
            if "-ios." in name:
                (self.assets / name).unlink()
        result = self.create()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)

    def test_legacy_full_manifest_without_policy_field_is_still_valid(self) -> None:
        self.assertEqual(self.create().returncode, 0)
        data = json.loads(self.manifest.read_text())
        data.pop("excluded_platforms")
        self.manifest.write_text(json.dumps(data))
        result = subprocess.run(
            [str(MANIFEST), "verify", "--tag", TAG, "--manifest", str(self.manifest),
             "--asset-dir", str(self.assets), "--require-name", names(TAG)[0]],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

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


class IOSExcludedManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        self.expected = names(IOS_EXCLUDED_TAG, include_ios=False)
        for name in self.expected:
            (self.assets / name).write_bytes(name.encode())
        self.manifest = self.root / f"iris-chat-{IOS_EXCLUDED_TAG}-manifest.json"

    def command(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([str(MANIFEST), *args], capture_output=True, text=True)

    def create(self) -> subprocess.CompletedProcess[str]:
        return self.command("create", "--tag", IOS_EXCLUDED_TAG, "--commit", "abc123",
                            "--asset-dir", str(self.assets), "--out", str(self.manifest))

    def test_explicit_inventory_verifies_and_stages_without_ios_or_losing_history(self) -> None:
        result = self.create()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(self.manifest.read_text())
        self.assertEqual(data["excluded_platforms"], ["ios"])
        self.assertEqual({asset["name"] for asset in data["assets"]}, set(self.expected))
        self.assertEqual(len(data["assets"]), 11)
        result = self.command("verify", "--tag", IOS_EXCLUDED_TAG, "--manifest", str(self.manifest),
                              "--asset-dir", str(self.assets), "--require-name", self.expected[0])
        self.assertEqual(result.returncode, 0, result.stderr)
        inventory = self.command("list-assets", "--tag", IOS_EXCLUDED_TAG)
        self.assertEqual(set(inventory.stdout.splitlines()), set(self.expected))
        previous = self.root / "release-tree" / TAG
        previous.mkdir(parents=True)
        (previous / "release.json").write_bytes(b"previous immutable release")
        notes = self.root / "notes.md"
        notes.write_text("Release without iOS.\n")
        stage = previous.parent / IOS_EXCLUDED_TAG
        result = self.command("stage-hashtree", "--tag", IOS_EXCLUDED_TAG,
                              "--manifest", str(self.manifest), "--asset-dir", str(self.assets),
                              "--notes", str(notes), "--out-dir", str(stage),
                              "--published-at", "2026-10-08T10:00:00Z")
        self.assertEqual(result.returncode, 0, result.stderr)
        release = json.loads((stage / "release.json").read_text())
        self.assertEqual({asset["name"] for asset in release["assets"]},
                         {*self.expected, self.manifest.name})
        self.assertNotIn("ios", {asset["platform"] for asset in release["assets"]})
        self.assertEqual((stage / "assets" / self.manifest.name).read_bytes(), self.manifest.read_bytes())
        self.assertEqual((previous / "release.json").read_bytes(), b"previous immutable release")

    def test_policy_only_disables_ios_for_its_explicit_tag(self) -> None:
        for tag, platform, expected in [(IOS_EXCLUDED_TAG, "ios", "false"),
                                       (IOS_EXCLUDED_TAG, "macos", "true"),
                                       (TAG, "ios", "true"), ("v2026.10.8.3", "ios", "false"),
                                       ("v2026.10.8.3", "macos", "true"),
                                       ("v2026.10.7", "ios", "true")]:
            with self.subTest(tag=tag, platform=platform):
                result = self.command("platform-enabled", "--tag", tag, "--platform", platform)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), expected)
        rejected = self.command("require-platform", "--tag", IOS_EXCLUDED_TAG, "--platform", "ios")
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("ios artifacts are excluded", rejected.stderr)

    def test_exclusion_still_rejects_missing_other_platforms_and_unexpected_ios(self) -> None:
        for name in self.expected:
            with self.subTest(missing=name):
                path = self.assets / name
                original = path.read_bytes()
                path.unlink()
                result = self.create()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("missing", result.stderr)
                path.write_bytes(original)
        for name in names(IOS_EXCLUDED_TAG):
            if "-ios." not in name:
                continue
            path = self.assets / name
            path.write_bytes(b"unexpected iOS build")
            result = self.create()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unexpected", result.stderr)
            path.unlink()

    def test_manifest_cannot_invent_or_hide_exclusions(self) -> None:
        self.assertEqual(self.create().returncode, 0)
        original = json.loads(self.manifest.read_text())
        for exclusions in (None, [], ["ios", "android"], ["linux"]):
            data = dict(original)
            if exclusions is None:
                data.pop("excluded_platforms")
            else:
                data["excluded_platforms"] = exclusions
            self.manifest.write_text(json.dumps(data))
            result = self.command("verify", "--tag", IOS_EXCLUDED_TAG,
                                  "--manifest", str(self.manifest), "--asset-dir", str(self.assets),
                                  "--require-name", self.expected[0])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("platform policy mismatch", result.stderr)

    def test_policy_rejects_other_platform_exclusions_and_invalid_values(self) -> None:
        script = self.root / "scripts" / "release-manifest.py"
        script.parent.mkdir()
        script.write_bytes(MANIFEST.read_bytes())
        script.chmod(0o755)
        policy = self.root / "release-platforms.json"
        for exclusions in (["android"], ["ios", "linux"], ["ios", "ios"], "ios", None):
            policy.write_text(json.dumps({"schema_version": 1, "excluded_platforms_by_tag": {
                IOS_EXCLUDED_TAG: exclusions,
            }}))
            result = subprocess.run([str(script), "list-assets", "--tag", IOS_EXCLUDED_TAG],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("only ios may be excluded", result.stderr)


if __name__ == "__main__":
    unittest.main()
