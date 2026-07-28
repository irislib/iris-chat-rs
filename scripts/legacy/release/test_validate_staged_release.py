#!/usr/bin/env python3

import json
import subprocess
import tempfile
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "validate-staged-release.py"


class ValidateStagedReleaseTests(unittest.TestCase):
    def make_stage(self, root: Path) -> Path:
        stage = root / "v1.2.3"
        assets = stage / "assets"
        assets.mkdir(parents=True)
        artifact = assets / "app.zip"
        artifact.write_bytes(b"release artifact")
        (stage / "release.json").write_text(
            json.dumps(
                {
                    "tag": "v1.2.3",
                    "commit": "abc123",
                    "assets": [
                        {
                            "name": artifact.name,
                            "path": f"assets/{artifact.name}",
                            "size": artifact.stat().st_size,
                        }
                    ],
                }
            )
        )
        return stage

    def validate(self, stage: Path, tag: str = "v1.2.3", commit: str = "abc123"):
        return subprocess.run(
            [str(VALIDATOR), str(stage), tag, commit],
            capture_output=True,
            text=True,
        )

    def test_accepts_exact_manifest_and_prints_asset_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stage = self.make_stage(Path(directory))
            result = self.validate(stage)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.strip(), str((stage / "assets" / "app.zip").resolve())
            )

    def test_rejects_wrong_commit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stage = self.make_stage(Path(directory))
            result = self.validate(stage, commit="other")

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("commit", result.stderr)

    def test_rejects_changed_asset(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stage = self.make_stage(Path(directory))
            (stage / "assets" / "app.zip").write_bytes(b"changed")
            result = self.validate(stage)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("size", result.stderr)

    def test_rejects_asset_outside_assets_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stage = self.make_stage(Path(directory))
            manifest = json.loads((stage / "release.json").read_text())
            manifest["assets"][0]["path"] = "../outside"
            (stage / "release.json").write_text(json.dumps(manifest))
            result = self.validate(stage)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("path", result.stderr)

    def test_rejects_non_cli_executable_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            stage = self.make_stage(Path(directory))
            manifest = json.loads((stage / "release.json").read_text())
            manifest["assets"][0]["executable"] = "iris/iris"
            (stage / "release.json").write_text(json.dumps(manifest))
            result = self.validate(stage)

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("executable", result.stderr)


if __name__ == "__main__":
    unittest.main()
