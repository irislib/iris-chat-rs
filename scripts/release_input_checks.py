"""Pure release-input contracts, collected by test_release_workflow.py."""
from pathlib import Path
import json
import hashlib
import os
import re
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseInputPinTests(unittest.TestCase):
    def test_release_external_actions_require_full_commit_pins(self) -> None:
        for filename in ("release.yml", "build-artifacts.yml", "release-contract.yml"):
            workflow = (ROOT / ".github/workflows" / filename).read_text()
            uses = re.findall(r"^\s*(?:- )?uses: ([^\s#]+)", workflow, re.MULTILINE)
            self.assertTrue(uses)
            for action in uses:
                with self.subTest(workflow=filename, action=action):
                    if action.startswith("./"):
                        self.assertEqual(action, "./.github/workflows/build-artifacts.yml")
                    else:
                        self.assertRegex(action, r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")
            # A reused checkout anchor must still resolve to a pinned definition.
            if "*checkout_build" in workflow:
                self.assertRegex(workflow, r"&checkout_build\n\s+name:[^\n]+\n\s+uses: actions/checkout@[0-9a-f]{40}")


    def test_release_contract_runs_when_reviewed_build_inputs_change(self) -> None:
        contract = (ROOT / ".github/workflows/release-contract.yml").read_text()
        # Both PR and default-branch pushes must validate changes to these pins
        # and the installers consuming them, even without another source edit.
        for path in (".github/workflows/release-contract.yml", "scripts/release-build-inputs.*",
                     "global.json", "linux/Dockerfile", "linux/build-inputs/**",
                     "linux/scripts/**", "scripts/docker_stage_common.sh", "scripts/cli-release",
                     "scripts/linux-release", "scripts/test_cli_install_docker",
                     "scripts/windows-build-local.ps1", "scripts/ios-build", "scripts/macos-build"):
            with self.subTest(path=path):
                self.assertEqual(contract.count(f'      - "{path}"'), 2)

    def test_core_release_gate_uses_verified_snapshot_and_selected_native_tools(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        gate = workflow.split("\n  verify:\n", 1)[1].split("\n  mesh-resource:\n", 1)[0]
        install = gate.split("      - name: Install native dependencies and identify compiler\n", 1)[1]
        install = install.split("\n      - name:", 1)[0]
        self.assertIn("set -Eeuo pipefail", install)
        self.assertNotIn("apt-get", install)
        self.assertIn("sudo bash linux/scripts/pinned-apt.sh ubuntu-noble cmake libdbus-1-dev pkg-config pkgconf pkgconf-bin", install)
        # Pinned package versions do not control ambient /usr/local/bin tools.
        for variable, executable in (("CMAKE", "/usr/bin/cmake"), ("PKG_CONFIG", "/usr/bin/pkg-config")):
            self.assertIn(f'echo "{variable}={executable}" >> "$GITHUB_ENV"', install)
        self.assertLess(gate.index("pinned-apt.sh ubuntu-noble"), gate.index("scripts/verify.sh fast"))
        script = textwrap.dedent(install.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixtures = {
                "sudo": '#!/bin/sh\nprintf "%s\\n" "$*" >> "$NATIVE_CALLS"\nexit "$NATIVE_EXIT"\n',
                "rustc": '#!/bin/sh\nprintf "%s\\n" "rustc fixture; no compiler executed"\n',
                "sha256sum": '#!/usr/bin/env python3\nimport hashlib, sys\nprint(hashlib.sha256(sys.stdin.buffer.read()).hexdigest()+"  -")\n',
            }
            for name, content in fixtures.items():
                path = root / name
                path.write_text(content)
                path.chmod(0o755)
            for code in (0, 1):
                with self.subTest(snapshot_exit=code):
                    exported, output, calls = (root / f"{name}-{code}" for name in ("env", "output", "calls"))
                    env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                               GITHUB_ENV=str(exported), GITHUB_OUTPUT=str(output),
                               NATIVE_CALLS=str(calls), NATIVE_EXIT=str(code))
                    result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                            capture_output=True, text=True, timeout=5)
                    self.assertEqual(calls.read_text().strip(),
                                     "bash linux/scripts/pinned-apt.sh ubuntu-noble cmake libdbus-1-dev pkg-config pkgconf pkgconf-bin")
                    if code:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertFalse(exported.exists())
                        self.assertFalse(output.exists())
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(exported.read_text().splitlines(),
                                         ["CMAKE=/usr/bin/cmake", "PKG_CONFIG=/usr/bin/pkg-config"])
                        self.assertRegex(output.read_text(), r"^identity=[0-9a-f]{64}\n$")


    def test_every_native_cache_restore_and_save_binds_reviewed_input_bytes(self) -> None:
        release = (ROOT / ".github/workflows/release.yml").read_text()
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        common = ["scripts/release-build-inputs.py", "scripts/release-build-inputs.json"]
        apt = ["linux/scripts/pinned-apt.sh", "linux/build-inputs/apt-snapshots.tsv"]
        policies = {
            "verify": (release, common + apt),
            "rust-package": (ci, common + apt),
            "android": (builds, common + apt + ["scripts/android-release-inputs.py", "scripts/android-release-inputs.json"]),
            "ios": (builds, common), "macos": (builds, common),
            "windows": (builds, common + ["global.json"]),
            "cli": (builds, common + ["scripts/docker_stage_common.sh", "linux/scripts/pinned-apt.sh", "linux/build-inputs/**"]),
        }
        def render(template, changed=None):
            def value(match):
                expression = match.group(1)
                if expression.startswith("hashFiles("):
                    paths = re.findall(r"'([^']+)'", expression)
                    payload = [(path, "changed" if path == changed else "original") for path in paths]
                    return hashlib.sha256(json.dumps(payload).encode()).hexdigest()
                return "complete" if "'complete'" in expression else "fixture"
            return re.sub(r"\$\{\{ (.*?) \}\}", value, template)
        for job, (workflow, inputs) in policies.items():
            body = re.split(r"\n  (?=\S)", workflow.split(f"\n  {job}:\n", 1)[1], maxsplit=1)[0]
            keys = re.findall(r"^          key: (.+)$", body, re.MULTILINE)
            if job == "rust-package":
                self.assertEqual(keys[-1], "${{ steps.cargo-cache.outputs.cache-primary-key }}")
                self.assertEqual(len(keys), 2)
                keys = keys[:-1]  # Save uses the restore action's exact pinned key.
            prefixes = []
            for match in re.finditer(r"^          restore-keys: (.+)$", body, re.MULTILINE):
                if match.group(1) == "|":
                    for line in body[match.end():].lstrip("\n").splitlines():
                        if not line.startswith("            "):
                            break
                        prefixes.append(line.strip())
                else:
                    prefixes.append(match.group(1))
            self.assertTrue(keys, job)
            self.assertTrue(prefixes, job)
            for template in [*keys, *prefixes]:
                for path in inputs:
                    with self.subTest(job=job, input=path, template=template):
                        self.assertIn(f"'iris-chat-rs/{path}'", template)
            # GitHub selects a cache by prefix: every restore must accept same
            # input bytes and reject each changed installer/manifest independently.
            for prefix in prefixes:
                self.assertTrue(render(keys[0]).startswith(render(prefix)), job)
                for path in inputs:
                    self.assertFalse(render(keys[0]).startswith(render(prefix, "iris-chat-rs/" + path)), (job, path))
            if job == "verify":
                self.assertEqual(len(keys), 2)  # Restore and partial/complete save.
                self.assertEqual(len(prefixes), 3)
                self.assertTrue(all(value.startswith("rust-release-v2-") for value in [*keys, *prefixes]))


    def test_package_verification_uses_reviewed_inputs_and_stops_on_install_failure(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        package = workflow.split("\n  rust-package:\n", 1)[1].split("\n  rust:\n", 1)[0]
        self.assertNotIn("rustup", package)
        self.assertNotIn("apt-get", package)
        for action in re.findall(r"^\s*uses: ([^\s#]+)", package, re.MULTILINE):
            self.assertRegex(action, r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")
        install = package.split("      - name: Install checksum-verified package compiler\n", 1)[1]
        install = install.split("\n      - name:", 1)[0]
        self.assertIn('scripts/release-build-inputs.py --install --work "$RUNNER_TEMP/iris-package-rust"', install)
        receipt = package.split("      - name: Retain reviewed package compiler inputs\n", 1)[1]
        receipt = receipt.split("\n      - name:", 1)[0]
        self.assertIn("if: always()", receipt)
        self.assertIn("path: ${{ runner.temp }}/iris-package-rust/receipt.json", receipt)
        self.assertLess(package.index("Install checksum-verified package compiler"),
                        package.index("Restore package compiler cache"))
        record = package.split("      - name: Record selected package source\n", 1)[1]
        record = record.split("\n      - name:", 1)[0]
        for path in ("scripts/release-build-inputs.py", "scripts/release-build-inputs.json",
                     "linux/scripts/pinned-apt.sh", "linux/build-inputs/apt-snapshots.tsv"):
            self.assertIn(path, record)
        native = package.split("      - name: Install package dependencies and identify compiler cache\n", 1)[1]
        native = native.split("\n      - name:", 1)[0]
        self.assertIn("id: toolchain", native)
        script = textwrap.dedent(native.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "work/rust-package").mkdir(parents=True)
            fixtures = {
                "sudo": '#!/bin/sh\nprintf "%s\\n" "$*" >> "$NATIVE_CALLS"\nexit "$NATIVE_EXIT"\n',
                "rustc": '#!/bin/sh\nprintf "%s\\n" "verified rustc fixture"\n',
                "cargo": '#!/bin/sh\nprintf "%s\\n" "verified cargo fixture"\n',
                "sha256sum": '#!/usr/bin/env python3\nimport hashlib, pathlib, sys\nprint(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest()+"  -")\n',
            }
            for name, content in fixtures.items():
                path = root / name
                path.write_text(content)
                path.chmod(0o755)
            for code in (17, 0):
                with self.subTest(snapshot_exit=code):
                    exported, output, calls = (root / f"{name}-{code}" for name in ("env", "output", "calls"))
                    env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                               GITHUB_ENV=str(exported), GITHUB_OUTPUT=str(output),
                               NATIVE_CALLS=str(calls), NATIVE_EXIT=str(code))
                    result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                            capture_output=True, text=True, timeout=5)
                    self.assertEqual(calls.read_text().strip(),
                                     "bash linux/scripts/pinned-apt.sh ubuntu-noble cmake libdbus-1-dev pkg-config pkgconf pkgconf-bin")
                    self.assertEqual(result.returncode, code, result.stderr)
                    if code:
                        self.assertFalse(exported.exists())
                        self.assertFalse(output.exists())
                        self.assertFalse((root / "work/rust-package/rustc.txt").exists())
                    else:
                        self.assertEqual(exported.read_text().splitlines(),
                                         ["CMAKE=/usr/bin/cmake", "PKG_CONFIG=/usr/bin/pkg-config"])
                        self.assertRegex(output.read_text(), r"^identity=[0-9a-f]{64}\n$")
                        self.assertEqual((root / "work/rust-package/cargo.txt").read_text(),
                                         "verified cargo fixture\n")
        verify = package.split("      - name: Verify selected package without publishing\n", 1)[1]
        verify = verify.split("\n      - name:", 1)[0]
        self.assertIn('cargo package --manifest-path "$PACKAGE_CRATE/Cargo.toml" --locked', verify)
        self.assertIn("sha256sum --check --status work/rust-package/source-sha256.txt", verify)
        save = package.split("      - name: Preserve compiled package dependencies after failure\n", 1)[1]
        self.assertIn("key: ${{ steps.cargo-cache.outputs.cache-primary-key }}", save)


    def test_release_rust_and_binding_steps_enforce_committed_locks(self) -> None:
        for filename in ("scripts/ios-build", "scripts/macos-build", "scripts/linux-release"):
            script = (ROOT / filename).read_text()
            commands = re.findall(r"^\s*cargo (?:build|run)[^\n]+", script, re.MULTILINE)
            release = [command for command in commands if "--release" in command]
            self.assertTrue(release)
            for command in release:
                with self.subTest(script=filename, command=command):
                    self.assertIn("--locked", command.split())
            if filename != "scripts/linux-release":
                generator = next(command for command in commands if "uniffi-bindgen/Cargo.toml" in command)
                self.assertIn("--locked", generator.split())
                # Debug builds retain their existing profile and dependency-update behavior.
                self.assertRegex(script, r"else\n\s+cargo build(?: --lib)?\n")
        android = (ROOT / "android/app/build.gradle.kts").read_text()
        for start, end in (("val buildRustHostDebug", "val generateRustBindings"),
                           ("val generateRustBindings", "fun registerRustAndroidTask"),
                           ("fun registerRustAndroidTask", "val buildRustAndroidDebug")):
            task = android.split(start, 1)[1].split(end, 1)[0]
            argv = re.search(r"(?:commandLine|mutableListOf)\(\s*cargoBinary.absolutePath,(.*?)\n\s*\)", task, re.DOTALL)
            self.assertIsNotNone(argv, start)
            self.assertIn('"--locked"', argv.group(1), start)
        # Release uses debug host bindings too: locking only --release is insufficient.
        self.assertIn("dependsOn(buildRustHostDebug)", android)
        self.assertIn("dependsOn(generateRustBindings)", android)


    def test_release_requires_pinned_read_only_source_verifier_before_attestation(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        release = workflow.split("\n  release:\n", 1)[1]
        self.assertIn("    timeout-minutes: 45\n", release)
        checkout = release.split("      - name: Checkout pinned public-source verifier\n", 1)[1]
        checkout = checkout.split("\n      - name:", 1)[0]
        self.assertIn("repository: mmalmi/haps", checkout)
        self.assertIn("ref: 3f70a6e71c579045ebc38006c27df9851bd7dde8", checkout)
        self.assertIn("path: .release-source-verifier", checkout)
        self.assertIn("persist-credentials: false", checkout)
        gate = release.split("      - name: Require complete public Hashtree source history\n", 1)[1]
        gate = gate.split("\n      - name:", 1)[0]
        self.assertIn("SOURCE_COMMIT: ${{ needs.validate.outputs.sha }}", gate)
        self.assertIn("--read-server https://cdn.iris.to", gate)
        self.assertIn("--read-server https://upload.iris.to", gate)
        self.assertEqual(gate.count("--read-server"), 2)
        self.assertIn("--clone-timeout 1800", gate)
        self.assertNotIn("if:", gate)
        self.assertNotIn("continue-on-error", gate)
        self.assertNotIn("secrets.", gate)
        self.assertNotIn("--publish", gate)
        self.assertNotIn("--key-file", gate)
        self.assertLess(release.index("Create immutable release manifest"),
                        release.index("Require complete public Hashtree source history"))
        self.assertLess(release.index("Require complete public Hashtree source history"),
                        release.index("Attest release build provenance"))
        self.assertLess(release.index("Attest release build provenance"),
                        release.index("uses: softprops/action-gh-release@"))


    def test_public_source_workflow_stops_on_wrong_bindings_or_incomplete_remote_history(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        step = workflow.split("      - name: Require complete public Hashtree source history\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n      - name:", 1)[0])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / ".release-source-verifier/scripts"
            tools.mkdir(parents=True)
            (root / "artifacts").mkdir()
            (root / "haps-release.json").write_text('{"source":"htree://npub1fixture/chat"}')
            (tools / "bundle.py").write_text(textwrap.dedent("""\
                import json, os, sys
                from pathlib import Path
                with open(os.environ['SOURCE_CALLS'], 'a') as log:
                    log.write(json.dumps(['bundle', *sys.argv[1:]]) + '\\n')
                if os.environ.get('SOURCE_FAILURE') == 'helper-checksum':
                    sys.exit('Hashtree archive checksum mismatch')
                args = sys.argv[1:]
                assert args[args.index('--target') + 1] == 'x86_64-unknown-linux-gnu'
                Path(args[args.index('--destination') + 1], 'libexec').mkdir(parents=True)
                """))
            (tools / "verify-public-source.py").write_text(textwrap.dedent("""\
                import json, os, sys
                from pathlib import Path
                args = sys.argv[1:]
                with open(os.environ['SOURCE_CALLS'], 'a') as log:
                    log.write(json.dumps(['verify', *args]) + '\\n')
                assert args[args.index('--commit') + 1] == os.environ['SOURCE_COMMIT']
                assert Path(args[args.index('--config') + 1]).resolve() == Path('haps-release.json').resolve()
                assert Path(args[args.index('--helpers') + 1]).is_dir()
                assert [args[i + 1] for i, value in enumerate(args) if value == '--read-server'] == ['https://cdn.iris.to', 'https://upload.iris.to']
                assert args[args.index('--clone-timeout') + 1] == '1800'
                if os.environ.get('SOURCE_FAILURE'):
                    sys.exit(os.environ['SOURCE_FAILURE'])
                """))
            git = root / "git"
            git.write_text(textwrap.dedent("""\
                #!/usr/bin/env python3
                import os, sys
                assert sys.argv[1] == 'rev-parse'
                assert sys.argv[2] in ('HEAD', 'refs/tags/' + os.environ['RELEASE_TAG'] + '^{commit}')
                print(os.environ['CHECKOUT_COMMIT' if sys.argv[2] == 'HEAD' else 'TAG_COMMIT'])
                """))
            git.chmod(0o755)
            revision = "a" * 40
            tag = "v2026.10.27"
            cases = ("success", "checkout", "tag", "manifest-commit", "manifest-tag",
                     "helper-checksum", "missing-commit", "shallow-history", "missing-parent", "clone-timeout")
            for index, case in enumerate(cases):
                with self.subTest(case=case):
                    calls = root / f"calls-{index}"
                    temporary = root / f"runner-{index}"
                    env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                               SOURCE_CALLS=str(calls), SOURCE_COMMIT=revision, RELEASE_TAG=tag,
                               CHECKOUT_COMMIT="b" * 40 if case == "checkout" else revision,
                               TAG_COMMIT="b" * 40 if case == "tag" else revision,
                               GITHUB_WORKSPACE=str(root), RUNNER_TEMP=str(temporary),
                               SOURCE_FAILURE=case if case in cases[5:] else "")
                    manifest = {"tag": "v2026.10.26" if case == "manifest-tag" else tag,
                                "commit": "b" * 40 if case == "manifest-commit" else revision}
                    (root / f"artifacts/iris-chat-{tag}-manifest.json").write_text(json.dumps(manifest))
                    result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                            capture_output=True, text=True, timeout=10)
                    commands = [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []
                    if case == "success":
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual([command[0] for command in commands], ["bundle", "verify"])
                    else:
                        self.assertNotEqual(result.returncode, 0, case)
                        expected = [] if case in cases[1:5] else ["bundle"] if case == "helper-checksum" else ["bundle", "verify"]
                        self.assertEqual([command[0] for command in commands], expected)
