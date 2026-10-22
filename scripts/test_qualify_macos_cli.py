import copy
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("qualify_macos_cli", ROOT / "scripts/qualify-macos-cli.py")
QUALIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUALIFY)

DESCRIPTION = "/tmp/iris: Mach-O 64-bit executable arm64"
LIBRARIES = "/tmp/iris:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n"
LOAD_COMMANDS = "Load command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 1\n    minos 14.0\n      sdk 15.0\n"


class MacCliQualificationTests(unittest.TestCase):
    def test_accepts_native_arm64_system_runtime(self):
        result = QUALIFY.checked_macho(DESCRIPTION, LIBRARIES, LOAD_COMMANDS)
        self.assertEqual(result, {
            "architecture": "arm64", "minimum_macos": "14.0",
            "system_libraries": ["/usr/lib/libSystem.B.dylib"],
        })

    def test_accepts_legacy_macos_deployment_command(self):
        result = QUALIFY.checked_macho(
            DESCRIPTION, LIBRARIES,
            "Load command 9\n      cmd LC_VERSION_MIN_MACOSX\n  cmdsize 16\n  version 11.0\n      sdk 15.0\n")
        self.assertEqual(result["minimum_macos"], "11.0")

    def test_rejects_wrong_architecture_or_universal_binary(self):
        for description in (DESCRIPTION.replace("arm64", "x86_64"), "ELF 64-bit executable",
                            "Mach-O universal binary with 2 architectures: " + DESCRIPTION):
            with self.subTest(description=description), self.assertRaises(RuntimeError):
                QUALIFY.checked_macho(description, LIBRARIES, LOAD_COMMANDS)

    def test_rejects_external_or_unresolved_runtime_libraries(self):
        for libraries in ("/tmp/iris:\n", LIBRARIES.replace("/usr/lib/", "/opt/homebrew/lib/"),
                          LIBRARIES.replace("/usr/lib/", "@rpath/")):
            with self.subTest(libraries=libraries), self.assertRaises(RuntimeError):
                QUALIFY.checked_macho(DESCRIPTION, libraries, LOAD_COMMANDS)

    def test_rejects_missing_wrong_platform_or_future_deployment_target(self):
        for commands in ("", LOAD_COMMANDS.replace("platform 1", "platform 2"),
                         LOAD_COMMANDS.replace("minos 14.0", "minos 16.0"),
                         LOAD_COMMANDS.replace("minos 14.0", "minos unavailable"),
                         LOAD_COMMANDS + LOAD_COMMANDS):
            with self.subTest(commands=commands), self.assertRaises(RuntimeError):
                QUALIFY.checked_macho(DESCRIPTION, LIBRARIES, commands)

    def test_retains_exact_normal_development_artifact_contract(self):
        artifact = {"reason": "compiler-artifact", "target": {"name": "iris", "kind": ["bin"]},
                    "features": [], "profile": {"opt_level": "0", "debuginfo": 2,
                    "debug_assertions": True, "overflow_checks": True, "test": False}}
        self.assertEqual(QUALIFY.checked_artifact([artifact]), artifact)
        for field, value in (("opt_level", "3"), ("debuginfo", 0), ("debug_assertions", False),
                             ("overflow_checks", False), ("test", True)):
            changed = copy.deepcopy(artifact)
            changed["profile"][field] = value
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                QUALIFY.checked_artifact([changed])
        for records in ([], [artifact, artifact], [{**artifact, "features": ["stack-fixture"]}],
                        [{**artifact, "target": {"name": "iris", "kind": ["lib"]}}]):
            with self.subTest(records=records), self.assertRaises(RuntimeError):
                QUALIFY.checked_artifact(records)

    def test_workflow_binds_exact_admitted_source_and_fixed_toolchain(self):
        workflow = (ROOT / ".github/workflows/macos-cli-qualification.yml").read_text()
        self.assertEqual(QUALIFY.COMMIT, "0bcc193ecd43e3740291ea6d774290c84dcada46")
        self.assertEqual(QUALIFY.TREE, "5f4fd4c6285fe919d68a3ff8a4b6a4241ff3c6d5")
        for binding in (f"SOURCE_COMMIT: {QUALIFY.COMMIT}", f"SOURCE_TREE: {QUALIFY.TREE}",
                        f"IRIS_BUILD_GIT_SHA: {QUALIFY.COMMIT}", f"ref: {QUALIFY.COMMIT}",
                        "RUSTUP_TOOLCHAIN: 1.95.0", "CARGO_BUILD_JOBS: '2'", "timeout-minutes: 40"):
            self.assertIn(binding, workflow)
        self.assertIn("branches: [codex/macos-cli-invite-fix-0bcc193]", workflow)
        self.assertNotIn("branches: [codex/macos-cli-invite-fix-0bcc193]",
                         (ROOT / ".github/workflows/linux-cli-qualification.yml").read_text())


if __name__ == "__main__":
    unittest.main()
