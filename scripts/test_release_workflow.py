#!/usr/bin/env python3

from pathlib import Path
import json
import os
import plistlib
import sys
import tempfile
import re
import shutil
import subprocess
import textwrap
import unittest

from release_input_checks import ReleaseInputPinTests
from ci_release_cache_checks import MainReleaseCacheTests


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_protocol_scope_keeps_default_and_tagged_gates_complete(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        rust = workflow.split("\n  rust:\n", 1)[1].split("\n  android:\n", 1)[0]
        self.assertIn("inputs.scope == 'protocol'", rust)
        self.assertIn("core, protocol, android", workflow)
        lint = rust.split("      - name: Rust lint including test targets\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("crates=(core chat-protocol protocol-ffi)", lint)
        self.assertIn('if [[ "$VALIDATION_SCOPE" == protocol ]]; then crates=(chat-protocol); fi', lint)
        default = rust.split("      - name: Rust tests\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: inputs.scope != 'protocol'", default)
        self.assertIn('IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT="$PWD/work/private-controls/native-private-controls.json" ./scripts/test_rust.sh\n', default)
        self.assertIn("test -s work/private-controls/native-private-controls.json", default)
        fixture = rust.split("      - name: Upload native private-control interoperability fixture\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: inputs.scope != 'protocol'", fixture)
        protocol = rust.split("      - name: Protocol tests\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: inputs.scope == 'protocol'", protocol)
        self.assertIn("set -Eeuo pipefail", protocol)
        self.assertIn('IRIS_PROTOCOL_TEST_OUTPUT_DIR="$PWD/work/protocol-checks"', protocol)
        self.assertIn("./scripts/test_rust.sh --protocol-only 2>&1 | tee work/protocol-checks/protocol-tests.log", protocol)
        self.assertNotIn("native-private-controls", protocol)
        for binding in ("git rev-parse HEAD", "git rev-parse HEAD^{tree}", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "source-sha256.txt"):
            self.assertIn(binding, protocol)
        upload = rust.split("      - name: Retain protocol test evidence\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: ${{ always() && inputs.scope == 'protocol' }}", upload)
        self.assertIn("path: iris-chat-rs/work/protocol-checks/", upload)
        package = rust.split("      - name: Verify protocol package\n", 1)[1].split("      - name:", 1)[0]
        self.assertNotIn("        if:", package)
        self.assertIn("cargo package --manifest-path chat-protocol/Cargo.toml --locked", package)
        self.assertEqual((ROOT / "scripts/test_rust.sh").read_text().count("<<'BATCHED_CPU_RECEIPT'"), 1)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        fast = (ROOT / "scripts/test_fast.sh").read_text()
        self.assertIn("scripts/verify.sh fast", release)
        self.assertNotIn("--protocol-only", release + fast)
        self.assertEqual(fast.count('run_step "Rust tests" "${ROOT_DIR}/scripts/test_rust.sh"'), 2)

    def test_ios_units_run_independently_after_successful_build(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        ios = workflow.split("\n  ios:\n", 1)[1].split("\n  windows-ui:\n", 1)[0]
        self.assertIn("id: ios-build", ios)
        self.assertIn("inputs.scope == 'ios-unit'", ios)
        progress = ios.split("      - name: iOS device linking progress test\n", 1)[1]
        self.assertTrue(progress.startswith("        if: inputs.scope != 'ios-unit'\n"))
        units = ios.split("      - name: iOS native composer, clipboard, and timeline tests\n", 1)[1]
        self.assertTrue(units.startswith("        if: ${{ !cancelled() && steps.ios-build.outcome == 'success' }}\n"))
        self.assertIn("selectors=(-only-testing:IrisChatTests -only-testing:NotificationServiceTests)", units)
        self.assertIn('if [[ "$IOS_VALIDATION_SCOPE" != ios-unit ]]; then', units)
        self.assertIn('-only-testing:IrisChatUITests/ComposerCompletionUITests', units)
        cached = ios.split('python3 scripts/reuse_ios_core_cache.py --producer-run "$IOS_CORE_SEED_RUN"', 1)[1]
        cached = cached.split('          else\n', 1)[0]
        self.assertIn('source scripts/build_common.sh', cached)
        self.assertIn('load_release_env "$PWD"', cached)
        self.assertLess(cached.index('resolve_shared_build_metadata "$PWD"'), cached.index('xcodegen generate'))

    def test_linux_release_cache_tracks_container_and_real_target(self) -> None:
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        linux = workflow.split("\n  linux:\n", 1)[1].split("\n  cli:\n", 1)[0]
        prepare = linux.split("      - name: Prepare Linux build cache identity\n", 1)[1]
        prepare = prepare.split("\n      - name:", 1)[0]
        cache = linux.split("      - name: Cache Linux release dependencies\n", 1)[1]
        cache = cache.split("\n      - name:", 1)[0]
        self.assertRegex(cache, r"uses: actions/cache@[0-9a-f]{40} # v6")
        self.assertIn("path: ${{ env.APP_DIR }}/linux/target/release", cache)
        for value in ("runner.os", "runner.arch", "steps.linux-cache.outputs.toolchain",
                      "iris-chat-rs/linux/Cargo.lock", "iris-chat-rs/**/Cargo.toml",
                      "iris-chat-rs/**/build.rs", "iris-chat-rs/linux/Dockerfile",
                      "iris-chat-rs/linux/.dockerignore", "iris-chat-rs/linux/scripts/**",
                      "iris-chat-rs/**/.cargo/config*", "iris-chat-rs/**/rust-toolchain*",
                      "iris-chat-rs/scripts/linux-release", "iris-chat-rs/scripts/build_common.sh",
                      "iris-chat-rs/scripts/docker_stage_common.sh"):
            self.assertIn(value, cache)
        key = next(line.split("key: ", 1)[1] for line in cache.splitlines() if "key: " in line)
        restore = cache.split("restore-keys: ", 1)[1].strip()
        self.assertEqual(key.rsplit("-${{ hashFiles('iris-chat-rs/linux/Cargo.lock') }}", 1)[0] + "-", restore)
        self.assertNotIn("~/.cargo", cache)  # The container's Cargo home is not mounted.
        self.assertNotIn("dist/", cache)
        self.assertNotIn("cache-hit", linux)
        self.assertIn("./scripts/linux-release --skip-docker-build", linux)
        self.assertLess(linux.index("Prepare Linux build cache identity"), linux.index("Cache Linux release dependencies"))
        self.assertLess(linux.index("Cache Linux release dependencies"), linux.index("Build and verify Linux artifacts"))
        release = (ROOT / "scripts/linux-release").read_text()
        self.assertIn('-v "$ROOT:/workspace/$REPO_BASENAME:cached"', release)
        self.assertIn('cd "/workspace/$IRIS_REPO_BASENAME/linux"', release)
        self.assertIn("cargo build --release", release)
        self.assertIn("BIN_PATH=target/release/iris-chat", release)

        # Run the actual workflow preparation with Docker replaced at its boundary.
        script = textwrap.dedent(prepare.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            docker = temp / "docker"
            docker.write_text(textwrap.dedent("""\
                #!/usr/bin/env python3
                import json, os, sys
                with open(os.environ['DOCKER_CALLS'], 'a') as log:
                    log.write(json.dumps(sys.argv[1:]) + '\\n')
                if sys.argv[1] == 'run':
                    if os.environ.get('FAIL_DOCKER') == '1':
                        sys.exit(37)
                    print(os.environ['CONTAINER_IDENTITY'])
                """))
            docker.chmod(0o755)
            keys = []
            for index, identity in enumerate(("rustc-A cargo-A packages-A", "rustc-A cargo-A packages-A",
                                              "rustc-B cargo-A packages-A", "rustc-A cargo-A packages-B")):
                calls = temp / f"calls-{index}"
                output = temp / f"output-{index}"
                env_file = temp / f"env-{index}"
                env = dict(os.environ, PATH=f"{temp}:{os.environ['PATH']}",
                           DOCKER_CALLS=str(calls), CONTAINER_IDENTITY=identity,
                           GITHUB_OUTPUT=str(output), GITHUB_ENV=str(env_file),
                           IRIS_LINUX_BUILD_IMAGE="cache-fixture:ci", IRIS_LINUX_DOCKER_PLATFORM="linux/amd64",
                           FAIL_DOCKER="0")
                result = subprocess.run(["bash", "-c", script], cwd=ROOT, env=env,
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                keys.append(output.read_text().strip())
                self.assertRegex(keys[-1], r"^toolchain=[0-9a-f]{64}$")
                self.assertEqual(env_file.read_text(), "IRIS_LINUX_BUILD_IMAGE=cache-fixture:ci\nIRIS_LINUX_DOCKER_PLATFORM=linux/amd64\n")
                commands = [json.loads(line) for line in calls.read_text().splitlines()]
                self.assertEqual(len(commands), 3)  # buildx probe, image build, identity query
                self.assertEqual(commands[1][:3], ["buildx", "build", "--load"])
                self.assertIn("INSTALL_CARGO_WATCH=0", commands[1])
                self.assertEqual(commands[1][commands[1].index("-t") + 1], "cache-fixture:ci")
                self.assertEqual(commands[2][:6], ["run", "--rm", "--platform", "linux/amd64", "--entrypoint", "/bin/bash"])
                self.assertEqual(commands[2][6], "cache-fixture:ci")
                for value in ("rustc -vV", "cargo -V", "dpkg-query -W", "sort"):
                    self.assertIn(value, commands[2][-1])
            self.assertEqual(keys[0], keys[1])
            self.assertNotEqual(keys[0], keys[2])
            self.assertNotEqual(keys[0], keys[3])
            output.unlink()
            env["FAIL_DOCKER"] = "1"
            result = subprocess.run(["bash", "-c", script], cwd=ROOT, env=env,
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 37)
            self.assertFalse(output.exists())

    def test_tagged_platform_policy_skips_only_ios_build_and_rejects_apple_distribution(self) -> None:
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        metadata = builds.split("\n  metadata:\n", 1)[1].split("\n  android:\n", 1)[0]
        self.assertIn('build_ios=true', metadata)
        self.assertIn('[[ -f release-platforms.json', metadata)
        self.assertIn('platform-enabled --tag "$requested_tag" --platform ios', metadata)
        self.assertIn('echo "build_ios=$build_ios"', metadata)
        ios = builds.split("\n  ios:\n", 1)[1].split("\n  macos:\n", 1)[0]
        self.assertIn("if: needs.metadata.outputs.build_ios == 'true'", ios)
        for platform in ("android", "macos", "windows", "linux", "cli"):
            job = re.split(r"\n  (?=\S)", builds.split(f"\n  {platform}:\n", 1)[1], maxsplit=1)[0]
            self.assertNotIn("build_ios", job)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('scripts/release-manifest.py list-assets --tag "$RELEASE_TAG"', release)
        apple = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        check = apple.index("- name: Check iOS release policy")
        self.assertLess(check, apple.index("- name: Download and verify exact release IPA"))
        self.assertLess(check, apple.index("- name: Configure App Store Connect authentication"))
        self.assertIn('source/scripts/release-manifest.py require-platform', apple)
        self.assertIn('--tag "$RELEASE_TAG" --platform ios', apple)
        self.assertEqual((ROOT / ".github/workflows/release-contract.yml").read_text().count(
            '      - "release-platforms.json"'), 2)
        script = textwrap.dedent(metadata.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            for index, (ref, expected) in enumerate([
                ("v2026.10.8.2", "false"), ("refs/tags/v2026.10.8.2", "false"),
                ("v2026.10.8.3", "false"), ("refs/tags/v2026.10.8.3", "false"),
                ("v2026.10.7", "true"), ("main", "true"),
            ]):
                with self.subTest(ref=ref):
                    output = Path(directory) / str(index)
                    env = dict(os.environ, REQUESTED_REF=ref, GITHUB_OUTPUT=str(output))
                    result = subprocess.run(["bash", "-c", script], cwd=ROOT, env=env,
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(f"build_ios={expected}\n", output.read_text())

    def test_ios_filtering_requires_entitlement_in_profile_and_signed_extension(self) -> None:
        verifier = ROOT / "scripts/verify-ios-notification-filtering.py"
        key = "com.apple.developer.usernotifications.filtering"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "entitlements.plist"
            for wrapped in (False, True):
                for value in (None, False, "true", 1, True):
                    with self.subTest(profile=wrapped, value=repr(value)):
                        entitlements = {} if value is None else {key: value}
                        document = {"Entitlements": entitlements} if wrapped else entitlements
                        path.write_bytes(plistlib.dumps(document))
                        result = subprocess.run(
                            [sys.executable, str(verifier), str(path)], capture_output=True, text=True
                        )
                        self.assertEqual(result.returncode, 0 if value is True else 1)
            path.write_bytes(b"not a plist")
            result = subprocess.run([sys.executable, str(verifier), str(path)], capture_output=True)
            self.assertEqual(result.returncode, 1)
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertEqual(workflow.count("python3 scripts/verify-ios-notification-filtering.py"), 2)
        self.assertIn('"$app/PlugIns/NotificationService.appex"', workflow)
        source = plistlib.loads((ROOT / "ios/NotificationService/NotificationService.entitlements").read_bytes())
        self.assertIs(source.get(key), True)
        for gate in ("scripts/ios-build", ".github/workflows/ci.yml"):
            self.assertIn("-only-testing:NotificationServiceTests", (ROOT / gate).read_text())

    def test_selected_ios_ui_tests_name_existing_classes_and_methods(self) -> None:
        tests = {}
        for path in (ROOT / "ios/UITests").glob("*.swift"):
            current = None
            for line in path.read_text().splitlines():
                declaration = re.match(r"(?:final )?class (\w+)\s*:", line)
                if declaration:
                    current = declaration.group(1)
                    tests.setdefault(current, set())
                method = re.match(r"\s+func (test\w+)\s*\(", line)
                if current and method:
                    tests[current].add(method.group(1))
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        selectors = re.findall(r"-only-testing:IrisChatUITests/([\w/]+)", workflow)
        self.assertTrue(selectors)
        for selector in selectors:
            parts = selector.split("/")
            with self.subTest(selector=selector):
                self.assertIn(parts[0], tests)
                if len(parts) == 2:
                    self.assertIn(parts[1], tests[parts[0]])

    def test_tagged_gates_keep_existing_cache_locations_with_input_bound_keys(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        gate = workflow.split("\n  mesh-resource:\n", 1)[0]
        self.assertIn("CARGO_TARGET_DIR: ${{ github.workspace }}/cargo-target", gate)
        self.assertIn("working-directory: iris-chat-rs", gate)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        restore = gate.split("- name: Restore release compiler cache\n", 1)[1].split("- name:", 1)[0]
        self.assertRegex(restore, r"uses: actions/cache/restore@[0-9a-f]{40} # v6")
        for path in ("~/.cargo/registry", "~/.cargo/git", "${{ github.workspace }}/cargo-target"):
            self.assertIn(path, ci)
            self.assertIn(path, restore)
        self.assertIn("scripts/verify.sh fast", gate)
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        for job in ("android", "macos"):
            with self.subTest(platform=job):
                paths = []
                for text in (ci, builds):
                    body = re.split(r"\n  (?=\S)", text.split(f"\n  {job}:\n", 1)[1], maxsplit=1)[0]
                    cache = body.split("      - name: Cache Cargo", 1)[1].split("\n      - ", 1)[0]
                    paths.append(cache.split("          path: |\n", 1)[1].split("          key:", 1)[0])
                    self.assertNotIn("cache-hit", body)
                if job == "android":
                    # Tagged Android builds isolate generated Gradle state and
                    # restore only the real dependency cache, never symlinks.
                    self.assertIn("~/.gradle/caches\n", paths[0])
                    self.assertIn("~/.gradle/wrapper\n", paths[0])
                    self.assertEqual(
                        paths[1],
                        paths[0].replace("~/.gradle/caches", "~/.cache/iris-android-gradle/caches/modules-2")
                        .replace("            ~/.gradle/wrapper\n", ""),
                    )
                else:
                    self.assertEqual(paths[0], paths[1])
        # ReleaseInputPinTests checks every restore prefix against changed input
        # bytes. Cargo cache locations stay intact; unsafe broad keys do not.

    def test_release_cache_retains_partial_then_complete_without_per_retry_entries(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        gate = workflow.split("\n  verify:\n", 1)[1].split("\n  mesh-resource:\n", 1)[0]
        restore = gate.split("      - name: Restore release compiler cache\n", 1)[1].split("\n      - name:", 1)[0]
        save = gate.split("      - name: Preserve release compiler cache after tests\n", 1)[1].split("\n      - name:", 1)[0]
        self.assertRegex(save, r"uses: actions/cache/save@[0-9a-f]{40} # v6")
        for step in (restore, save):
            self.assertIn("runner.arch", step)
            self.assertIn("steps.toolchain.outputs.identity", step)
            self.assertIn("needs.validate.outputs.sha", step)
            self.assertNotIn("github.run_id", step)
            self.assertNotIn("github.run_attempt", step)
            self.assertNotIn("github.sha", step)  # Dispatch may select a different immutable tag.
        self.assertEqual(restore.split("          path: |\n", 1)[1].split("          key:", 1)[0],
                         save.split("          path: |\n", 1)[1].split("          key:", 1)[0])
        self.assertIn("${{ needs.validate.outputs.sha }}-complete", restore)
        self.assertIn("${{ steps.release-gate.outcome == 'success' && 'complete' || 'partial' }}", save)
        self.assertIn('identity=$(rustc -Vv | sha256sum | cut -d\' \' -f1)', gate)
        self.assertLess(gate.index("Upload populated discovery CPU and privacy evidence"),
                        gate.index("Preserve release compiler cache after tests"))
        self.assertNotIn("continue-on-error:", gate)

        # Evaluate the actual workflow predicate against failure/cancellation and
        # restored-cache states, without invoking GitHub, Cargo or a cache writer.
        expression = re.search(r"        if: \$\{\{ (.+) \}\}", save).group(1)
        source = "a" * 40
        partial = "rust-release-v2-Linux-X64-compiler-inputs-lock-" + source + "-partial"
        cases = [
            ("cold pass", False, "success", "success", "false", "", True),
            ("cold failure", False, "success", "failure", "false", "", True),
            ("same-input fallback failure", False, "success", "failure", "false", "rust-release-v2-Linux-X64-compiler-inputs-older-lock", True),
            ("partial becomes complete", False, "success", "success", "false", partial, True),
            ("repeat partial failure", False, "success", "failure", "false", partial, False),
            ("other source partial", False, "success", "failure", "false", partial.replace(source, "b" * 40), True),
            ("complete hit", False, "success", "success", "true", "complete", False),
            ("complete hit then test failure", False, "success", "failure", "true", "complete", False),
            ("cancelled", True, "success", "failure", "false", "", False),
            ("restore failed", False, "failure", "success", "false", "", False),
            ("gate skipped", False, "success", "skipped", "false", "", False),
        ]
        for name, cancelled, restored, outcome, hit, matched, expected in cases:
            with self.subTest(case=name):
                predicate = expression
                for field, value in {
                    "cancelled()": cancelled, "steps.cargo-cache.outcome": restored,
                    "steps.release-gate.outcome": outcome,
                    "steps.cargo-cache.outputs.cache-hit": hit,
                    "steps.cargo-cache.outputs.cache-matched-key": matched,
                    "needs.validate.outputs.sha": source,
                }.items():
                    predicate = predicate.replace(field, repr(value))
                predicate = re.sub(r"!(?!=)", "not ", predicate).replace("&&", "and").replace("||", "or")
                actual = eval(predicate, {"__builtins__": {},
                                         "endsWith": lambda value, suffix: value.endswith(suffix),
                                         "format": lambda pattern, value: pattern.format(value)})
                self.assertEqual(actual, expected)

    def test_release_only_edits_keep_contract_checks_without_native_rebuilds(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        expected = [
            '      - "**/*.md"',
            '      - "haps.toml"',
            '      - ".github/workflows/release.yml"',
            '      - ".github/workflows/release-checks.yml"',
            '      - "scripts/test_release_workflow.py"',
            '      - "scripts/apple_connect_api.py"',
            '      - "scripts/test_apple_connect_api.py"',
            '      - "scripts/run_public_link_phone.py"',
            '      - "scripts/test_ios_harness_retry_contract.py"',
            '      - "scripts/android_upgrade_release.py"',
            '      - "scripts/android_upgrade_state.py"',
            '      - "scripts/android_upgrade_ui.py"',
            '      - "scripts/check_android_upgrade_receipt.py"',
            '      - "scripts/test_android_release_upgrade.py"',
            '      - "scripts/test_android_upgrade_harness.py"',
            '      - "test-fixtures/android-upgrade/releases.json"',
            '      - "scripts/ios_adhoc_prepare.py"',
            '      - "scripts/test_ios_adhoc_prepare.py"',
            '      - "scripts/ios_ordinary_relaunch.py"',
            '      - "scripts/qualify_ios_ordinary_relaunch.py"',
            '      - "scripts/test_ios_ordinary_relaunch.py"',
            '      - "scripts/ios_relaunch_adapters.py"',
            '      - "scripts/test_ios_relaunch_adapters.py"',
            '      - "scripts/ios_relaunch_lifecycle.py"',
            '      - "scripts/test_ios_relaunch_lifecycle.py"',
            '      - "scripts/ios_quality_observer.py"',
            '      - "scripts/ios_quality_trace.py"',
            '      - "scripts/qualify_ios_quality_observer.py"',
            '      - "scripts/test_ios_quality_observer.py"',
            '      - "scripts/ios_runtime_acceptance.py"',
            '      - "scripts/test_ios_runtime_acceptance.py"',
            '      - "scripts/test_ios_runtime_workflow.py"',
            '      - "scripts/ios_early_background_observer.py"',
            '      - "scripts/test_ios_early_background_observer.py"',
            '      - "scripts/ios_notification_subscription_reader.py"',
            '      - "scripts/test_ios_notification_subscription_reader.py"',
            '      - "scripts/ios_ostrace_capture.py"',
            '      - "scripts/ios_ostrace_logger.py"',
            '      - "scripts/test_ios_ostrace_capture.py"',
            '      - "scripts/ios_voip_background_wake.py"',
            '      - "scripts/test_ios_voip_background_wake.py"',
            '      - "scripts/ios_warm_lifecycle.py"',
            '      - "scripts/test_ios_warm_lifecycle.py"',
        ]
        # Haps recipes have dedicated checks below. Changes to any native app or
        # build input must still trigger the original full platform checks.
        for block in ci.split("    paths-ignore:\n")[1:]:
            lines = []
            for line in block.splitlines():
                if not line.startswith("      - "):
                    break
                lines.append(line)
            self.assertEqual(lines, expected)
        self.assertEqual(ci.count("    paths-ignore:\n"), 2)
        quick = (ROOT / ".github/workflows/release-checks.yml").read_text()
        for path in (".github/workflows/ci.yml", ".github/workflows/release.yml", ".github/workflows/release-checks.yml",
                     "haps.toml", "haps-release.json", "scripts/validate-haps-manifest.py", "scripts/test_haps_manifest.py",
                     "scripts/test_release_workflow.py", "scripts/apple_connect_api.py", "scripts/test_apple_connect_api.py",
                     "scripts/run_public_link_phone.py", "scripts/test_ios_harness_retry_contract.py"):
            self.assertEqual(quick.count(f'      - "{path}"'), 2)
        for script in ("test_release_workflow.py", "test_release_notes.py", "test_build_common.py", "test_apple_connect_api.py",
                       "test_ios_harness_retry_contract.py", "test_haps_manifest.py"):
            self.assertIn(f"python3 scripts/{script}", quick)
        self.assertIn("node --version", quick)
        self.assertIn('"scripts/render-release-notes.py"', quick)
        self.assertIn('"--channel", "validate"', quick)

    def test_standalone_apple_helpers_run_contract_without_native_builds(self) -> None:
        import fnmatch
        native = (ROOT / ".github/workflows/ci.yml").read_text()
        contract = (ROOT / ".github/workflows/release-contract.yml").read_text()
        standalone = (
            "scripts/ios_adhoc_prepare.py", "scripts/test_ios_adhoc_prepare.py",
            "scripts/ios_ordinary_relaunch.py", "scripts/qualify_ios_ordinary_relaunch.py",
            "scripts/test_ios_ordinary_relaunch.py", "scripts/ios_relaunch_adapters.py",
            "scripts/test_ios_relaunch_adapters.py", "scripts/ios_relaunch_lifecycle.py",
            "scripts/test_ios_relaunch_lifecycle.py", "scripts/ios_quality_observer.py",
            "scripts/ios_quality_trace.py", "scripts/qualify_ios_quality_observer.py",
            "scripts/test_ios_quality_observer.py", "scripts/ios_runtime_acceptance.py",
            "scripts/test_ios_runtime_acceptance.py", "scripts/test_ios_runtime_workflow.py",
            "scripts/ios_early_background_observer.py",
            "scripts/test_ios_early_background_observer.py",
            "scripts/ios_notification_subscription_reader.py",
            "scripts/test_ios_notification_subscription_reader.py",
            "scripts/ios_ostrace_capture.py",
            "scripts/ios_ostrace_logger.py",
            "scripts/test_ios_ostrace_capture.py",
            "scripts/ios_voip_background_wake.py",
            "scripts/test_ios_voip_background_wake.py",
            "scripts/ios_warm_lifecycle.py",
            "scripts/test_ios_warm_lifecycle.py",
        )
        native_inputs = (
            "scripts/native_lab.py", "scripts/test_native_lab.py", "scripts/test_fast.sh",
            "scripts/test_native_contract.sh", "scripts/ios-build", "scripts/macos-build",
            "ios/Sources/IrisApp.swift", "ios/UITests/FipsBlePhysicalUITests.swift",
            "macos/Sources/IrisApp.swift", "core/src/lib.rs", "protocol-ffi/src/lib.rs",
            "android/app/src/main/java/to/iris/chat/MainActivity.kt", "linux/src/main.rs",
            "windows/IrisChat/MainWindow.xaml.cs", ".github/workflows/ci.yml",
        )

        def patterns(workflow, event, key):
            body = re.search(rf"^  {event}:\n(.*?)(?=^  \S|\Z)", workflow, re.M | re.S).group(1)
            block = re.search(rf"^    {key}:\n((?:      - .+\n)+)", body, re.M).group(1)
            return [json.loads(line.removeprefix("      - ")) for line in block.splitlines()]

        for event in ("push", "pull_request"):
            ignored, included = patterns(native, event, "paths-ignore"), patterns(contract, event, "paths")
            for path in standalone:
                with self.subTest(event=event, helper=path):
                    self.assertTrue((ROOT / path).is_file())
                    self.assertIn(path, ignored)
                    self.assertIn(path, included)
            for path in native_inputs:
                with self.subTest(event=event, native_input=path):
                    self.assertFalse(any(fnmatch.fnmatchcase(path, rule) for rule in ignored))
                    changed = ["scripts/ios_quality_observer.py", path]
                    self.assertTrue(any(not any(fnmatch.fnmatchcase(item, rule) for rule in ignored)
                                        for item in changed))
        self.assertIn("scripts/test_release_publish_guard", contract)
        guard = (ROOT / "scripts/test_release_publish_guard").read_text()
        self.assertIn("unittest discover", guard)
        self.assertIn("-p 'test_*.py'", guard)

    def test_release_metadata_and_mixed_changes_select_the_required_workflows(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        quick = (ROOT / ".github/workflows/release-checks.yml").read_text()

        def paths(workflow, event, key):
            body = re.search(rf"^  {event}:\n(.*?)(?=^  \S|\Z)", workflow, re.M | re.S).group(1)
            block = re.search(rf"^    {key}:\n((?:      - .+\n)+)", body, re.M).group(1)
            return [json.loads(line.removeprefix("      - ")) for line in block.splitlines()]

        def matches(path, pattern):
            # These trigger lists contain exact paths and one recursive Markdown
            # pattern; reject a newly broadened pattern instead of guessing it.
            if pattern == "**/*.md":
                return path.endswith(".md")
            self.assertNotRegex(pattern, r"[*?\[!]")
            return path == pattern

        cases = [
            (["RELEASE_NOTES.md", "haps.toml"], False, True),
            (["haps.toml"], False, True),
            (["haps-release.json"], True, True),
            (["scripts/validate-haps-manifest.py"], True, True),
            (["scripts/test_haps_manifest.py"], True, True),
            (["haps.toml", "core/src/lib.rs"], True, True),
            (["haps.toml", "ios/Sources/IrisApp.swift"], True, True),
            (["android/app/src/main/java/to/iris/chat/MainActivity.kt"], True, False),
            (["scripts/linux-release"], True, False),
            ([".github/workflows/ci.yml"], True, True),
        ]
        for event in ("push", "pull_request"):
            ignored = paths(ci, event, "paths-ignore")
            included = paths(quick, event, "paths")
            for changed, native, metadata in cases:
                with self.subTest(event=event, changed=changed):
                    self.assertEqual(any(not any(matches(path, pattern) for pattern in ignored)
                                         for path in changed), native)
                    self.assertEqual(any(matches(path, pattern) for path in changed for pattern in included), metadata)

    @unittest.skipUnless(sys.version_info >= (3, 11), "Haps TOML checks require Python 3.11+")
    def test_release_metadata_step_rejects_stale_versions_and_unlocked_recipes(self) -> None:
        quick = (ROOT / ".github/workflows/release-checks.yml").read_text()
        script = textwrap.dedent(re.search(r"          python3 - <<'PY'\n(.*?)          PY", quick, re.S).group(1))
        original = (ROOT / "haps.toml").read_text()
        locked_recipe = '"--manifest-path", "linux/Cargo.toml", "--locked"'
        self.assertIn(locked_recipe, original)
        variants = [
            ("current", original, 0, "Validated 6 Haps package variants"),
            ("stale", original.replace('version = "', 'version = "9', 1), 1, "version"),
            ("unlocked", original.replace(locked_recipe, '"--manifest-path", "linux/Cargo.toml"', 1),
             1, "native locked Cargo"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            for name in ("haps-release.json", "RELEASE_NOTES.md", "scripts/render-release-notes.py",
                         "scripts/validate-haps-manifest.py"):
                shutil.copyfile(ROOT / name, root / name)
            for name, manifest, code, message in variants:
                with self.subTest(manifest=name):
                    (root / "haps.toml").write_text(manifest)
                    result = subprocess.run([sys.executable, "-c", script], cwd=root,
                                            capture_output=True, text=True, timeout=15)
                    self.assertEqual(result.returncode, code, result.stdout + result.stderr)
                    self.assertIn(message, result.stdout + result.stderr)

    def test_resource_gate_binds_tag_commit_and_blocks_publication(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        pin = "e34fa190b41a3ced5148e92b103418004bdd0b65"
        self.assertIn(f"irislib/iris-stack/.github/workflows/product-lab.yml@{pin}", workflow)
        self.assertIn(f"lab_rev: {pin}", workflow)
        self.assertIn("chat_rev: ${{ needs.validate.outputs.sha }}", workflow)
        self.assertIn("drive_rev: 7cb74966ddaecf90fb91b8f36a44ecc4bbda7b02", workflow)
        self.assertIn("htree_version: 0.2.148", workflow)
        release = workflow.split("\n  release:\n", 1)[1]
        self.assertIn("      - mesh-resource\n", release.split("    runs-on:", 1)[0])
        self.assertIn("pattern: iris-*-${{ needs.build.outputs.artifact_suffix }}", release)
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertIn("value: ${{ jobs.metadata.outputs.version_name }}-${{ jobs.metadata.outputs.short_sha }}", builds)

    def test_independent_release_work_starts_after_validation_and_all_gates_block_publish(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = dict(re.findall(r"(?ms)^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("\njobs:\n", 1)[1]))
        self.assertEqual(set(jobs), {"validate", "verify", "mesh-resource", "build", "release"})
        self.assertNotIn("    needs:", jobs["validate"])
        self.assertIn('echo "sha=$(git rev-parse HEAD)"', jobs["validate"])
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", jobs["validate"])
        for name in ("verify", "mesh-resource", "build"):
            with self.subTest(job=name):
                self.assertEqual(re.findall(r"^    needs: (.+)$", jobs[name], re.MULTILINE), ["validate"])
                self.assertNotIn("continue-on-error:", jobs[name])
        self.assertIn("ref: ${{ needs.validate.outputs.sha }}", jobs["verify"])
        self.assertIn("scripts/verify.sh fast", jobs["verify"])
        self.assertIn('IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT="$PWD/work/private-controls/native-private-controls.json"', jobs["verify"])
        self.assertIn("name: populated-discovery-cpu-and-privacy", jobs["verify"])
        self.assertIn("path: iris-chat-rs/work/private-controls/", jobs["verify"])
        publish = jobs["release"].split("    runs-on:", 1)[0]
        self.assertEqual(re.findall(r"^      - (.+)$", publish, re.MULTILINE),
                         ["validate", "verify", "build", "mesh-resource"])
        self.assertNotIn("    if:", publish)
        self.assertNotIn("continue-on-error:", jobs["release"])

    def test_focused_runner_installs_rustfmt_before_running_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        focused = workflow.split("\n  rust-focused:\n", 1)[1].split("\n  rust-package:\n", 1)[0]
        install = focused.split("      - name: Install Rust and identify compiler cache\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("rustup toolchain install stable --profile minimal --component rustfmt", install)
        self.assertIn("rustup default stable", install)
        self.assertLess(focused.index("Install Rust and identify compiler cache"),
                        focused.index("Build once and run exact tests"))
        self.assertNotIn("continue-on-error:", install)

    def test_release_validation_checks_shared_formatting_without_building(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        validate = workflow.split("\n  validate:\n", 1)[1].split("\n  verify:\n", 1)[0]
        step = validate.split("      - name: Check shared Rust formatting before builds\n", 1)[1]
        self.assertIn('CARGO_NET_OFFLINE: "true"', step)
        self.assertIn("set -Eeuo pipefail", step)
        self.assertIn('scripts/release-build-inputs.py --install --work "$RUNNER_TEMP/iris-release-rust" --component rustfmt', validate)
        self.assertNotIn("rustup", validate)
        self.assertEqual(re.findall(r"cargo fmt --manifest-path (\S+) --check", step),
                         ["chat-protocol/Cargo.toml", "core/Cargo.toml", "protocol-ffi/Cargo.toml"])
        self.assertNotIn("continue-on-error:", validate)
        self.assertNotIn("        if:", step)
        self.assertNotRegex(validate, r"cargo(?: \+stable)? (?:build|test|check|fetch|install)\b")
        self.assertNotIn("actions/cache", validate)
        self.assertNotIn("apt-get", validate)

    def test_hosted_release_gate_can_write_its_first_fixture_in_a_clean_checkout(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        step = workflow.split("      - name: Run hosted release gate\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n      - name:", 1)[0])
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            scripts = checkout / "scripts"
            scripts.mkdir()
            (scripts / "test_release_notes.py").write_text("")
            # Stand in for the first native fixture writer, which does not
            # depend on a different test creating its output directory.
            (scripts / "verify.sh").write_text(textwrap.dedent("""\
                #!/usr/bin/env python3
                import os
                from pathlib import Path
                Path(os.environ["IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT"]).write_text("fixture")
                """))
            for name in ("test_release_publish_guard", "test_homebrew_tap"):
                (scripts / name).write_text("#!/bin/sh\nexit 0\n")
            for name in ("verify.sh", "test_release_publish_guard", "test_homebrew_tap"):
                (scripts / name).chmod(0o755)
            result = subprocess.run(["bash", "-c", script], cwd=checkout,
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((checkout / "work/private-controls/native-private-controls.json").read_text(),
                             "fixture")

    def test_release_builds_manifest_and_attests_every_file(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("scripts/release-manifest.py create", workflow)
        self.assertIn("subject-path: artifacts/*", workflow)
        self.assertIn("git cat-file -t", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn('python3 scripts/validate-haps-manifest.py --tag "$RELEASE_TAG"', workflow)
        self.assertIn("gh release verify-asset", workflow)
        self.assertIn("--json isImmutable", workflow)
        self.assertIn("if: steps.existing.outputs.exists != 'true'", workflow)
        self.assertIn("--channel github", workflow)
        self.assertIn("--notes RELEASE_NOTES.md", workflow)
        self.assertNotIn("CHANGELOG.md", workflow)
        self.assertNotIn("ZAPSTORE_RELEASE_NOTES.md", workflow)

    def test_build_workflow_uploads_only_canonical_names(self) -> None:
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertIn("iris-chat-v${IRIS_APP_VERSION_NAME}-android-arm64.apk", workflow)
        self.assertIn("iris-v${IRIS_APP_VERSION_NAME}-${target}.tar.gz", workflow)
        upload_paths = "\n".join(
            line for line in workflow.splitlines() if "dist/android/" in line
        )
        self.assertNotIn("dist/android/*.apk", upload_paths)
        self.assertNotIn("latest", upload_paths)
        self.assertIn("runs-on: macos-26", workflow)

    def test_one_apple_workflow_reuses_exact_tagged_ipa(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        self.assertIn("- testflight", workflow)
        self.assertIn("- testflight-public", workflow)
        self.assertIn("- app-store", workflow)
        self.assertIn("group: ios-distribution-${{ inputs.tag }}", workflow)
        self.assertIn('ipa_name="iris-chat-${RELEASE_TAG}-ios.ipa"', workflow)
        self.assertIn("gh attestation verify", workflow)
        self.assertIn("gh release verify-asset", workflow)
        self.assertIn("--json isDraft,isImmutable,isPrerelease,url", workflow)
        self.assertIn("--commit \"$(git -C source rev-parse HEAD)\"", workflow)
        self.assertIn('expected_version="$(apple_marketing_version', workflow)
        self.assertIn('expected_build="$(semantic_version_code', workflow)
        self.assertIn("existing_build", workflow)
        self.assertIn("distribute_only", workflow)
        self.assertIn(
            "#{group_setting} must name at least one #{group_kind} group",
            workflow,
        )
        self.assertIn("get_edit_app_store_version", workflow)
        self.assertIn("completed_states", workflow)
        self.assertIn("attached_build = exact_version.build&.version", workflow)
        self.assertIn("submit_for_review: true", workflow)
        self.assertNotIn("./scripts/ios-release archive", workflow)
        self.assertFalse((ROOT / ".github/workflows/ios-testflight-upload.yml").exists())
        self.assertFalse((ROOT / ".github/workflows/android-release-apk.yml").exists())

    def test_app_store_retry_resumes_ready_review_submission(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        completed_states = workflow.split("completed_states = [", 1)[1].split("]", 1)[0]
        self.assertNotIn("READY_FOR_REVIEW", completed_states)

        resume_start = workflow.index(
            "if exact_version&.app_version_state == ready_state"
        )
        normal_start = workflow.index("edit_version = app.get_edit_app_store_version")
        build_check_start = workflow.index(
            "if exact_version &&", workflow.index("completed_states = [")
        )
        build_check = workflow[build_check_start:resume_start]
        resume_path = workflow[resume_start:normal_start]
        self.assertIn("exact_version.app_version_state == ready_state", build_check)
        self.assertIn("attached_build = exact_version.build&.version", build_check)
        self.assertIn("if attached_build != build_number", build_check)
        self.assertIn('includes: "appStoreVersionForReview"', resume_path)
        self.assertIn(
            "submission&.app_store_version_for_review&.id == exact_version.id",
            resume_path,
        )
        self.assertIn(
            "Spaceship::ConnectAPI::ReviewSubmissionItem.all(", resume_path
        )
        self.assertIn("review_submission_id: submission.id", resume_path)
        self.assertIn('includes: "appStoreVersion"', resume_path)
        self.assertIn("items.one?", resume_path)
        self.assertIn(
            "items.first.app_store_version&.id == exact_version.id", resume_path
        )
        self.assertNotIn("submission.items", resume_path)
        self.assertIn(
            "submitted = submission.submit_for_review(client: client)", resume_path
        )
        self.assertIn(
            "ReviewSubmissionState::WAITING_FOR_REVIEW", resume_path
        )
        self.assertIn("ReviewSubmissionState::IN_REVIEW", resume_path)
        self.assertIn(
            "submitted_states.include?(submitted&.state)", resume_path
        )
        self.assertEqual(resume_path.count("submit_for_review"), 1)
        self.assertNotIn("upload_to_app_store", resume_path)
        self.assertLess(
            resume_path.index("UI.user_error!"),
            resume_path.index("submitted = submission.submit_for_review"),
        )
        self.assertLess(
            resume_path.index("submitted_states.include?(submitted&.state)"),
            resume_path.index("UI.success"),
        )
        self.assertIn("next", resume_path[resume_path.index("UI.success") :])

    def test_app_store_retry_never_recreates_an_unhandled_exact_version(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        self.assertIn("if exact_version && !edit_version", workflow)
        self.assertIn("existing_version = !exact_version.nil?", workflow)
        self.assertNotIn("existing_version = !edit_version.nil?", workflow)

    @unittest.skipUnless(shutil.which("ruby"), "Ruby is needed to exercise the Fastfile")
    def test_testflight_routes_validate_audience_and_never_enter_app_store(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        fastfile = textwrap.dedent(
            workflow.split("<<'RUBY'\n", 1)[1].split("\n          RUBY", 1)[0]
        )
        # Run the production lane with read-only API stubs. Every publishing
        # action is recorded, including the pinned Pilot beta-review default.
        stub = r'''
          require "tempfile"
          module Deliver
            class UploadMetadata
            end
          end
          $LOADED_FEATURES << "deliver/upload_metadata.rb"
          class UserError < StandardError
          end
          module UI
            def self.user_error!(message)
              raise UserError, message
            end
            def self.success(message)
              $messages << message
            end
            def self.important(message)
              $messages << message
            end
          end
          Group = Struct.new(:name, :is_internal_group, :public_link_enabled)
          BetaDetail = Struct.new(:auto_notify_enabled, :external_build_state)
          FakeBuild = Struct.new(:id, :build_beta_detail)
          class FakeApp
            def id
              "test-app"
            end
            def get_beta_groups(client:)
              $events << :read_groups
              $groups
            end
            def get_app_store_versions(**)
              $events << :app_store
              raise "TestFlight entered the App Store route"
            end
          end
          module Spaceship
            module ConnectAPI
              module Token
                def self.create(**)
                  :test_token
                end
              end
              class Client
                def initialize(token:)
                end
                def test_flight_request_client
                  self
                end
                def post(path, body)
                  raise "Wrong notification endpoint" unless path == "v1/buildBetaNotifications"
                  expected = {data: {type: "buildBetaNotifications", relationships: {build: {data: {type: "builds", id: "exact-build"}}}}}
                  raise "Notifying wrong build" unless body == expected
                  $events << :activate_public_build
                end
              end
              module App
                def self.find(*, **)
                  FakeApp.new
                end
              end
              module Build
                def self.all(**options)
                  raise "Wrong exact version" unless options[:version] == "2026.10.100" && options[:build_number] == "1"
                  if $uploaded_options
                    $events << :read_public_status
                    return [] if $missing_build
                    return [FakeBuild.new("exact-build", $missing_detail ? nil : BetaDetail.new($auto_notify, $external_state))]
                  end
                  $existing_build ? [Object.new] : []
                end
                def self.get(client:, build_id:)
                  raise "Wrong build verification" unless build_id == "exact-build"
                  $events << :verify_activation
                  FakeBuild.new(build_id, BetaDetail.new(true, $activated_state))
                end
              end
              module Platform
                IOS = "IOS"
              end
            end
          end
          def default_platform(*)
          end
          def platform(*)
            yield
          end
          def lane(*, &block)
            $distribution_lane = block
          end
          def app_store_connect_api_key(**)
            :test_api_key
          end
          def upload_to_testflight(**options)
            $events << :testflight
            $uploaded_options = options
            # Fastlane 2.237.0 submits beta review for any supplied groups
            # unless this option explicitly overrides its true default.
            if options.fetch(:submit_beta_review, true) &&
               (options[:groups] || options[:distribute_external])
              $events << :beta_review
            end
            if options[:distribute_external] || options[:notify_external_testers] != false
              $events << :external_distribution_or_notification
            end
          end
          def upload_to_app_store(**)
            $events << :app_store
            raise "TestFlight called upload_to_app_store"
          end
          def sleep(*)
          end
          eval(STDIN.read, TOPLEVEL_BINDING, "Fastfile")
          ENV.update(
            "IRIS_ASC_AUTH_KEY_ID" => "test-key",
            "IRIS_ASC_AUTH_KEY_ISSUER_ID" => "test-issuer",
            "IRIS_ASC_AUTH_KEY_PATH" => "unused.p8",
            "IRIS_IOS_BUNDLE_ID" => "test.iris",
            "APP_VERSION" => "2026.10.100",
            "BUILD_NUMBER" => "1",
            "DISTRIBUTION_TARGET" => "testflight",
            "IPA_PATH" => "attested.ipa"
          )
          Tempfile.create("iris-testflight-notes") do |notes|
            notes.write("Tagged release notes\n")
            notes.flush
            ENV["RELEASE_NOTES_PATH"] = notes.path
            internal = [Group.new("Team", true), Group.new("QA", true)]
            public_groups = [Group.new("Public", false, true), Group.new("Preview", false, true)]
            audience_cases = {
              "testflight" => [
                [" Team, QA, Team ", internal, nil],
                ["Team, Missing", internal, "not found"],
                ["Team, Public", internal + public_groups, "internal"],
                ["Team", internal + [Group.new("Team", false, true)], "internal"],
                ["Unknown", [Group.new("Unknown", nil)], "internal"],
                [" , ", internal, "at least one internal group"]
              ],
              "testflight-public" => [
                [" Public, Preview, Public ", public_groups, nil],
                ["Public, Missing", public_groups, "not found"],
                ["Public, Team", public_groups + internal, "public external"],
                ["Public", public_groups + [Group.new("Public", true, true)], "public external"],
                ["Unknown", [Group.new("Unknown", nil, true)], "public external"],
                ["Closed", [Group.new("Closed", false, false)], "public link enabled"],
                ["Unknown", [Group.new("Unknown", false, nil)], "public link enabled"],
                [" , ", public_groups, "at least one public external group"]
              ]
            }
            [false, true].each do |existing|
              audience_cases.each do |target, cases|
                public_target = target == "testflight-public"
                ENV["DISTRIBUTION_TARGET"] = target
                cases.each do |configured, available, expected_error|
                  ENV["IRIS_TESTFLIGHT_GROUPS"] = public_target ? "Must not use internal groups" : configured
                  ENV["IRIS_TESTFLIGHT_PUBLIC_GROUPS"] = public_target ? configured : "Must not use public groups"
                  $groups = available
                  $existing_build = existing
                  $events = []
                  $messages = []
                  $auto_notify = true
                  $external_state = "IN_BETA_TESTING"
                  $uploaded_options = nil
                  error = nil
                  begin
                    $distribution_lane.call
                  rescue UserError => failure
                    error = failure.message
                  end
                  if expected_error
                    raise "Missing expected rejection: #{target}: #{configured}" unless error&.include?(expected_error)
                    raise "Mutation before rejection: #{$events}" unless ($events - [:read_groups]).empty?
                  else
                    raise error if error
                    expected_events = [:read_groups, :testflight]
                    expected_events += [:beta_review, :external_distribution_or_notification] if public_target
                    expected_events << :read_public_status if public_target
                    raise "Unexpected actions: #{$events}" unless $events == expected_events
                    options = $uploaded_options
                    wanted_groups = public_target ? ["Public", "Preview"] : ["Team", "QA"]
                    raise "Wrong groups" unless options[:groups] == wanted_groups
                    raise "Wrong release notes" unless options[:changelog] == "Tagged release notes"
                    raise "Public release must enable automatic distribution after approval" unless options[:notify_external_testers] == public_target
                    raise "Expired or rejected another build" if options[:expire_previous_builds] || options[:reject_build_waiting_for_review]
                    if existing
                      raise "Existing build was re-uploaded" unless options[:distribute_only] == true && !options.key?(:ipa)
                      raise "Existing build lacks explicit iOS platform" unless options[:app_platform] == "ios"
                    else
                      raise "New IPA was skipped" unless options[:ipa] == "attested.ipa" && !options.key?(:distribute_only)
                    end
                  end
                end
              end
            end
            ENV["DISTRIBUTION_TARGET"] = "testflight-public"
            ENV["IRIS_TESTFLIGHT_PUBLIC_GROUPS"] = "Public"
            $groups = public_groups
            [false, true].each do |existing|
              [
                ["IN_BETA_TESTING", true, "IN_BETA_TESTING", "available to testers", false],
                ["WAITING_FOR_BETA_REVIEW", true, nil, "awaiting Apple review", false],
                ["IN_BETA_REVIEW", true, nil, "awaiting Apple review", false],
                ["BETA_APPROVED", true, "IN_BETA_TESTING", "available to testers", false],
                ["READY_FOR_BETA_TESTING", true, "IN_BETA_TESTING", "available to testers", false],
                ["BETA_APPROVED", true, "BETA_APPROVED", "not available", true],
                ["BETA_REJECTED", true, nil, "not available", true],
                ["EXPIRED", true, nil, "not available", true],
                ["IN_BETA_TESTING", false, nil, "not enabled", true],
                ["missing-build", true, nil, "not found", true],
                ["missing-detail", true, nil, "not enabled", true]
              ].each do |state, auto_notify, activated, message, fails|
                $existing_build = existing
                $external_state = state
                $auto_notify = auto_notify
                $activated_state = activated
                $missing_build = state == "missing-build"
                $missing_detail = state == "missing-detail"
                $uploaded_options = nil
                $events = []
                $messages = []
                error = nil
                begin
                  $distribution_lane.call
                rescue UserError => failure
                  error = failure.message
                end
                raise "Wrong public result for #{state}: #{error || $messages}" unless fails ? error&.include?(message) : error.nil? && $messages.any? { |m| m.include?(message) }
                should_activate = ["BETA_APPROVED", "READY_FOR_BETA_TESTING"].include?(state)
                raise "Wrong activation count" unless $events.count(:activate_public_build) == (should_activate ? 1 : 0)
                raise "Repeated notification while waiting" if $events.count(:activate_public_build) > 1
                raise "Entered App Store" if $events.include?(:app_store)
              end
            end
            $uploaded_options = nil
            ENV["DISTRIBUTION_TARGET"] = "typo"
            $events = []
            begin
              $distribution_lane.call
              raise "Unknown target was accepted"
            rescue UserError => failure
              raise failure unless failure.message.include?("Unknown Apple distribution target")
              raise "Unknown target mutated Apple state" unless $events.empty?
            end
          end
        '''
        result = subprocess.run(
            [shutil.which("ruby"), "-e", textwrap.dedent(stub)],
            input=fastfile,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipUnless(shutil.which("ruby"), "Ruby is needed to exercise the Fastfile")
    def test_app_store_preserves_review_attachments_without_skipping_metadata(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        fastfile = textwrap.dedent(
            workflow.split("<<'RUBY'\n", 1)[1].split("\n          RUBY", 1)[0]
        )
        # Execute the actual embedded Fastfile against the pinned deliver method's
        # destructive default. The lane itself must never contact Apple in a test.
        stub = r'''
          module Deliver
            class UploadMetadata
              attr_reader :options, :events
              def initialize(options)
                @options = options
                @events = []
              end
              def upload(version)
                events << [:release_notes, options[:release_notes]]
                events << [:review_notes, options[:app_review_information]]
                review_attachment_file(version)
              end
              private
              def review_attachment_file(version)
                events << [:attachment, version]
                version.clear
                if options[:app_review_attachment_file]
                  raise "replacement failed" if options[:app_review_attachment_file] == "bad.pdf"
                  version << options[:app_review_attachment_file]
                end
              end
            end
          end
          $LOADED_FEATURES << "deliver/upload_metadata.rb"
          def default_platform(*)
          end
          def platform(*)
            yield
          end
          def lane(*)
          end
          eval(STDIN.read, TOPLEVEL_BINDING, "Fastfile")

          metadata = {
            release_notes: {"en-US" => "Release notes"},
            app_review_information: {notes: "Review instructions"}
          }
          [nil, "replacement.pdf"].each do |replacement|
            options = metadata.dup
            options[:app_review_attachment_file] = replacement if replacement
            uploader = Deliver::UploadMetadata.new(options)
            attachments = ["existing.pdf"]
            uploader.upload(attachments)
            expected = replacement ? [replacement] : ["existing.pdf"]
            raise "attachment was not preserved or replaced" unless attachments == expected
            expected_events = [[:release_notes, metadata[:release_notes]],
                               [:review_notes, metadata[:app_review_information]]]
            expected_events << [:attachment, attachments] if replacement
            raise "metadata or replacement path was skipped" unless uploader.events == expected_events
          end

          uploader = Deliver::UploadMetadata.new(metadata.merge(app_review_attachment_file: "bad.pdf"))
          begin
            uploader.upload(["existing.pdf"])
            raise "replacement error was swallowed"
          rescue RuntimeError => error
            raise unless error.message == "replacement failed"
          end
        '''
        result = subprocess.run(
            [shutil.which("ruby"), "-e", textwrap.dedent(stub)],
            input=fastfile,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("app_review_information", fastfile)
        self.assertIn("release_notes:", fastfile)
        self.assertNotIn("skip_metadata", fastfile)

    def test_only_supported_operator_entrypoint_is_active(self) -> None:
        self.assertTrue((ROOT / "scripts/distribute").exists())
        self.assertFalse((ROOT / "scripts/release").exists())
        self.assertTrue((ROOT / "scripts/legacy/release/local-build-and-publish").exists())


if __name__ == "__main__":
    unittest.main()
