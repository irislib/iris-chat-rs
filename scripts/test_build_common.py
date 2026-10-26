import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
COMMON = ROOT / "scripts/build_common.sh"


def version_code(version: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; semantic_version_code "$2"',
            "bash",
            str(COMMON),
            version,
        ],
        check=False,
        capture_output=True,
        text=True,
    )


class BuildVersionCodeTests(unittest.TestCase):
    def assert_code(self, version: str, expected: int) -> None:
        result = version_code(version)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(expected))

    def test_date_versions_pack_as_yyyymmdd_build(self) -> None:
        self.assert_code("2026.7.15", 2_026_071_500)
        self.assert_code("2026.7.15.1", 2_026_071_501)
        self.assert_code("2026.7.15.99", 2_026_071_599)

    def test_month_and_day_values_do_not_overlap(self) -> None:
        july = version_code("2026.7.14")
        august = version_code("2026.8.4")
        self.assertEqual(july.returncode, 0, july.stderr)
        self.assertEqual(august.returncode, 0, august.stderr)
        self.assertNotEqual(july.stdout, august.stdout)
        self.assertLess(int(july.stdout), int(august.stdout))

    def test_rejects_component_and_android_limit_overflow(self) -> None:
        for version in ("2026.7.15.100", "2026.100.1", "2100.0.1", "0.0.0"):
            with self.subTest(version=version):
                self.assertNotEqual(version_code(version).returncode, 0)


class AppleMarketingVersionTests(unittest.TestCase):
    def marketing_version(self, version: str) -> str:
        result = subprocess.run(
            ["bash", "-c", 'source "$1"; apple_marketing_version "$2"',
             "bash", str(COMMON), version],
            check=True, capture_output=True, text=True,
        )
        return result.stdout.strip()

    def test_historical_release_versions_stay_unchanged(self) -> None:
        for version, expected in [
            ("0.1.0", "0.1.0"),
            ("2026.7.28.1", "2026.7.28"),
            ("2026.9.8", "2026.9.8"),
        ]:
            with self.subTest(version=version):
                self.assertEqual(self.marketing_version(version), expected)

    def test_corrective_releases_advance_the_apple_visible_version(self) -> None:
        versions = [
            ("2026.9.8", "2026.9.8"),
            ("2026.9.8.1", "2026.9.801"),
            ("2026.9.8.99", "2026.9.899"),
            ("2026.9.9", "2026.9.900"),
            ("2026.10.1", "2026.10.100"),
            ("2027.1.1", "2027.1.100"),
        ]
        actual = [self.marketing_version(version) for version, _ in versions]
        self.assertEqual(actual, [expected for _, expected in versions])
        components = [tuple(map(int, value.split("."))) for value in actual]
        self.assertTrue(all(len(value) == 3 for value in components))
        self.assertTrue(all(a < b for a, b in zip(components, components[1:])))


class MacDevelopmentBuildTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = pathlib.Path(directory.name) / "source"
        for name in ("scripts", "core/uniffi-bindgen", "macos", "bin"):
            (self.root / name).mkdir(parents=True, exist_ok=True)
        for name in ("macos-build", "build_common.sh"):
            shutil.copy2(ROOT / "scripts" / name, self.root / "scripts" / name)
        self.log = self.root / "commands.jsonl"
        self.target = self.root.parent / "core cache"
        self.bindgen_target = self.root.parent / "bindgen cache"
        self.derived = self.root.parent / "Xcode cache"
        self.derived.mkdir()
        self.marker = self.derived / "retain-existing-cache"
        self.marker.write_text("keep")
        fake = self.root / "bin/fake"
        fake.write_text(f"#!{sys.executable}\n" + r'''
import json, os, pathlib, shutil, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if tool == "uname":
    print("Darwin" if args == ["-s"] else "arm64")
    raise SystemExit(0)
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps({"tool": tool, "args": args, "env": {
        key: os.environ.get(key) for key in (
            "CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_PROFILE_RELEASE_OPT_LEVEL",
            "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "IRIS_BUILD_CHANNEL")}}) + "\n")
def value(flag):
    return args[args.index(flag) + 1]
stage = ("core" if args[0] == "build" else "bindgen") if tool == "cargo" else (
    "framework" if args[0] == "-create-xcframework" else "swift") if tool == "xcodebuild" else tool
if os.environ.get("FAKE_FAIL") == stage:
    raise SystemExit(37)
if stage == "core":
    target = pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / "release"
    target.mkdir(parents=True, exist_ok=True)
    files = [target / ("libiris_chat_core." + suffix) for suffix in ("dylib", "a")]
    for path in files:
        path.write_text("core contents")
    print(json.dumps({"reason": "compiler-artifact", "target": {"name": "iris_chat_core"},
                      "filenames": [] if os.environ.get("FAKE_WRONG_TARGET") else list(map(str, files))}))
elif stage == "bindgen":
    output = pathlib.Path(value("--out-dir"))
    for name in ("IrisChatCore.swift", "IrisChatCoreFFI.h", "IrisChatCoreFFI.modulemap"):
        (output / name).write_text("generated " + name)
elif stage == "framework":
    output = pathlib.Path(value("-output")) / "macos-arm64"
    output.mkdir(parents=True)
    library = pathlib.Path(value("-library"))
    shutil.copy2(library, output / library.name)
    shutil.copytree(value("-headers"), output / "Headers")
elif stage == "swift":
    (pathlib.Path(value("-derivedDataPath")) / "Build/Products/Release/Iris Chat.app").mkdir(parents=True, exist_ok=True)
''')
        fake.chmod(0o755)
        for name in ("cargo", "uname", "xcodebuild", "xcodegen"):
            (self.root / "bin" / name).symlink_to(fake.name)
        # Only fixture tools and ordinary file/shell utilities are reachable.
        for name in ("bash", "dirname", "mkdir", "mktemp", "rm", "cmp", "cp"):
            (self.root / "bin" / name).symlink_to(shutil.which(name))
        (self.root / "bin/python3").symlink_to(sys.executable)
        self.env = {
            "PATH": str(self.root / "bin"), "HOME": str(self.root.parent),
            "FAKE_LOG": str(self.log), "CARGO_TARGET_DIR": str(self.target),
            "IRIS_MACOS_BINDGEN_TARGET_DIR": str(self.bindgen_target),
            "IRIS_MACOS_DEV_DERIVED_DATA": str(self.derived),
            "IRIS_RELEASE_ENV_FILE": "/dev/null", "IRIS_APP_VERSION_NAME": "0.1.0",
            "IRIS_BUILD_GIT_SHA": "fixture", "IRIS_BUILD_TIMESTAMP_UTC": "2026-01-01T00:00:00Z",
        }

    def run_build(self, **environment: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", str(self.root / "scripts/macos-build"), "macos-dev"],
            env={**self.env, **environment}, capture_output=True, text=True, timeout=15,
        )

    def commands(self) -> list[dict]:
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_one_optimized_native_core_build_and_incremental_unsigned_release_swift(self) -> None:
        release_env = self.root / "release.env"
        release_env.write_text("IRIS_BUILD_CHANNEL=release\n")
        result = self.run_build(IRIS_RELEASE_ENV_FILE=str(release_env))
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.commands()
        builds = [command for command in commands if command["tool"] == "cargo" and command["args"][0] == "build"]
        self.assertEqual(len(builds), 1)
        build = builds[0]
        for flag in ("--release", "--lib", "--locked", "--offline"):
            self.assertIn(flag, build["args"])
        self.assertNotIn("--target", build["args"])
        self.assertEqual(build["env"], {
            "CARGO_TARGET_DIR": str(self.target), "CARGO_BUILD_JOBS": "2",
            "CARGO_PROFILE_RELEASE_OPT_LEVEL": "3", "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS": "false",
            "IRIS_BUILD_CHANNEL": "development",
        })
        bindgen = next(command for command in commands if command["tool"] == "cargo" and command["args"][0] == "run")
        self.assertEqual(bindgen["args"][bindgen["args"].index("--target-dir") + 1], str(self.bindgen_target))
        self.assertEqual(bindgen["args"][bindgen["args"].index("--library") + 1], str(self.target / "release/libiris_chat_core.dylib"))
        swift = commands[-1]
        self.assertEqual(swift["tool"], "xcodebuild")
        self.assertEqual(swift["args"][swift["args"].index("-configuration") + 1], "Release")
        self.assertEqual(swift["args"][swift["args"].index("-derivedDataPath") + 1], str(self.derived))
        self.assertEqual(swift["args"][swift["args"].index("-jobs") + 1], "2")
        self.assertIn("CODE_SIGNING_ALLOWED=NO", swift["args"])
        self.assertEqual(self.marker.read_text(), "keep")
        self.assertFalse((self.root / "dist").exists())
        self.assertEqual(result.stdout.strip().splitlines()[-1], str(self.derived / "Build/Products/Release/Iris Chat.app"))

    def test_unchanged_bindings_framework_and_derived_data_are_preserved(self) -> None:
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        retained = list((self.root / "ios/Bindings").iterdir()) + [
            self.root / "macos/Frameworks/IrisChatCore.xcframework/macos-arm64/libiris_chat_core.a",
            self.marker,
        ]
        for path in retained:
            os.utime(path, (1_600_000_000, 1_600_000_000))
        self.log.write_text("")
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(path.stat().st_mtime == 1_600_000_000 for path in retained))
        self.assertFalse(any("-create-xcframework" in command["args"] for command in self.commands()))

    def test_stage_failure_stops_before_later_work(self) -> None:
        for stage, expected_last in (("core", "cargo"), ("bindgen", "cargo"),
                                     ("framework", "xcodebuild"), ("xcodegen", "xcodegen"),
                                     ("swift", "xcodebuild")):
            with self.subTest(stage=stage):
                self.log.write_text("")
                result = self.run_build(FAKE_FAIL=stage)
                self.assertEqual(result.returncode, 37, result.stderr)
                self.assertEqual(self.commands()[-1]["tool"], expected_last)
                self.assertEqual(self.marker.read_text(), "keep")
                self.assertFalse(list((self.root / "macos/.build").glob("dev-bindings.*")))

    def test_rejects_target_override_and_stale_native_artifacts(self) -> None:
        overridden = self.run_build(CARGO_BUILD_TARGET="aarch64-apple-ios")
        self.assertEqual(overridden.returncode, 2)
        self.assertEqual(self.commands(), [])
        stale = self.run_build(FAKE_WRONG_TARGET="1")
        self.assertNotEqual(stale.returncode, 0)
        self.assertIn("requires native core artifacts", stale.stderr)
        self.assertEqual(len(self.commands()), 1)


if __name__ == "__main__":
    unittest.main()
