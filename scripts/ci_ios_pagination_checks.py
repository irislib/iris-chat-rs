"""Pure contracts for the explicit, single-simulator pagination diagnosis."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest

from ci_release_cache_checks import evaluate, job, step


ROOT = Path(__file__).resolve().parents[1]
NARROW_SELECTORS = [
    "-only-testing:IrisChatTests/ChatTimelineHistoryDiagnosticsTests",
    "-only-testing:IrisChatUITests/ChatPaginationUITests/testMixedHeightHistoryPrependPreservesVisibleMessage",
]


class IosPaginationWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        cls.ios = job(cls.workflow, "ios", "windows-ui")

    def condition(self, body, scope, *, build="success", cancelled=False, event="workflow_dispatch"):
        expression = re.search(r"^\s+if: (.+)$", body, re.M).group(1)
        if expression.startswith("${{"):
            expression = expression[3:-2].strip()
        return evaluate(expression, {"inputs.scope": scope, "github.event_name": event,
                                     "steps.ios-build.outcome": build, "cancelled()": cancelled})

    def run_step(self, name, scope, seed=""):
        script = textwrap.dedent(step(self.ios, name).split("        run: |\n", 1)[1])
        script = script.replace("${{ inputs.scope }}", scope)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "bin").mkdir()
            recorder = "#!" + sys.executable + "\n" + textwrap.dedent("""\
                import json, os, sys
                from pathlib import Path
                with Path(os.environ['RECORDED_CALLS']).open('a') as stream:
                    stream.write(json.dumps({'program': Path(sys.argv[0]).name, 'args': sys.argv[1:],
                        'cargo_jobs': os.environ.get('CARGO_BUILD_JOBS'),
                        'rust_targets': os.environ.get('IRIS_IOS_RUST_TARGETS')}) + '\\n')
                """)
            for path in (root / "scripts/ios-build", root / "bin/xcodebuild"):
                path.write_text(recorder)
                path.chmod(0o700)
            destination = root / "scripts/ios-simulator-destination"
            destination.write_text("#!/bin/sh\nprintf '%s\\n' 'platform=iOS Simulator,id=fixture'\n")
            destination.chmod(0o700)
            calls = root / "calls.jsonl"
            env = dict(os.environ, PATH=str(root / "bin") + os.pathsep + os.environ["PATH"],
                       RECORDED_CALLS=str(calls), IOS_VALIDATION_SCOPE=scope, IOS_CORE_SEED_RUN=seed)
            result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=10)
            recorded = [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []
            return result, recorded

    def test_scope_selects_only_existing_ios_job(self):
        options = re.search(r"scope:\n.*?options: \[([^]]+)\]", self.workflow, re.S).group(1)
        self.assertIn("ios-pagination", [value.strip() for value in options.split(",")])
        bodies = re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                            self.workflow.split("\njobs:\n", 1)[1], re.M | re.S)
        for scope, expected in [
            ("ios-pagination", {"ios"}), ("ios", {"ios"}), ("ios-unit", {"ios"}),
            ("apple", {"ios", "macos"}),
            ("", {"rust", "android", "linux", "macos", "ios", "windows-ui", "cli-service"}),
            ("all", {"rust", "android", "linux", "macos", "ios", "windows-ui", "cli-service"}),
        ]:
            with self.subTest(scope=scope):
                selected = {name for name, body in bodies if self.condition(body, scope)}
                self.assertEqual(selected, expected)
        self.assertFalse(self.condition(self.ios, "ios-pagination", event="push"))

    def test_scope_skips_broad_and_link_steps_without_bypassing_build(self):
        broad = step(self.ios, "iOS native composer, clipboard, and timeline tests")
        linking = step(self.ios, "iOS device linking progress test")
        narrow = step(self.ios, "iOS pagination diagnostics")
        for scope in ["", "all", "ios", "ios-unit", "ios-pagination"]:
            with self.subTest(scope=scope):
                self.assertEqual(self.condition(linking, scope), scope not in {"ios-unit", "ios-pagination"})
                self.assertEqual(self.condition(broad, scope), scope != "ios-pagination")
                self.assertEqual(self.condition(narrow, scope), scope == "ios-pagination")
        for build, cancelled in [("failure", False), ("skipped", False), ("success", True)]:
            self.assertFalse(self.condition(narrow, "ios-pagination", build=build, cancelled=cancelled))
        self.assertIn("CARGO_TARGET_DIR: ${{ github.workspace }}/cargo-target", self.ios)
        self.assertIn("key: ios-${{ runner.os }}-${{ runner.arch }}-", self.ios)
        self.assertIn("if: always()", step(self.ios, "Upload iOS test results"))
        self.assertIn("path: iris-chat-rs/work/clipboard/", step(self.ios, "Upload iOS test results"))

    def test_exact_narrow_command_uses_one_simulator_and_two_jobs(self):
        result, calls = self.run_step("iOS pagination diagnostics", "ios-pagination")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        command = calls[0]
        self.assertEqual(command["program"], "xcodebuild")
        args = command["args"]
        self.assertEqual([arg for arg in args if arg.startswith("-only-testing:")], NARROW_SELECTORS)
        self.assertEqual(args[args.index("-destination") + 1], "platform=iOS Simulator,id=fixture")
        self.assertEqual(args[args.index("-parallel-testing-enabled") + 1], "NO")
        self.assertEqual(args[args.index("-jobs") + 1], "2")
        self.assertEqual(args[args.index("-resultBundlePath") + 1], "work/clipboard/ios-pagination.xcresult")
        self.assertEqual(args[-1], "test")
        self.assertNotIn("-skip-testing", " ".join(args))
        diagnostic_tests = (ROOT / "ios/Tests/ChatTimelineHistoryDiagnosticsTests.swift").read_text()
        self.assertEqual(len(re.findall(r"func test\w+\(", diagnostic_tests)), 3)

    def test_build_uses_existing_locked_wrapper_and_keeps_other_scopes(self):
        result, calls = self.run_step("iOS simulator build", "ios-pagination")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([(call["program"], call["args"][0]) for call in calls],
                         [("ios-build", "ios-xcframework"), ("ios-build", "ios-xcodeproj"), ("xcodebuild", "-project")])
        self.assertEqual(calls[0]["cargo_jobs"], "2")
        self.assertEqual(calls[0]["rust_targets"], "aarch64-apple-ios-sim")
        args = calls[-1]["args"]
        self.assertEqual(args[args.index("-jobs") + 1], "2")
        self.assertEqual(args[-1], "build")
        for scope in ["", "all", "ios", "ios-unit"]:
            result, calls = self.run_step("iOS simulator build", scope)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual([(call["program"], call["args"]) for call in calls],
                             [("ios-build", ["ios-build-sim"])])
        result, calls = self.run_step("iOS simulator build", "ios-pagination", seed="123")
        self.assertNotEqual(result.returncode, 0, "A pagination run cannot bypass existing core-cache provenance")
        self.assertEqual(calls, [])
        self.assertIn("cargo build --lib --locked", (ROOT / "scripts/ios-build").read_text())

    def test_existing_broad_selectors_are_unchanged(self):
        units = ["-only-testing:IrisChatTests", "-only-testing:NotificationServiceTests"]
        ui = ["-only-testing:IrisChatUITests/ComposerCompletionUITests",
              "-only-testing:IrisChatUITests/ChatTimelineUITests/testReopeningLongChatLandsAtBottom",
              "-only-testing:IrisChatUITests/IrisChatTimelineUITests/testSearchHitInSeededLongChatOpensInTimeline",
              "-only-testing:IrisChatUITests/ChatPaginationUITests"]
        for scope in ["", "all", "ios", "ios-unit"]:
            result, calls = self.run_step("iOS native composer, clipboard, and timeline tests", scope)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(calls), 1)
            selected = [arg for arg in calls[0]["args"] if arg.startswith("-only-testing:")]
            self.assertEqual(selected, units if scope == "ios-unit" else units + ui)


if __name__ == "__main__":
    unittest.main()
