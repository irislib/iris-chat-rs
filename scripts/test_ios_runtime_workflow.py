#!/usr/bin/env python3
"""Exercise the real promotion shell with GitHub replaced at its boundary."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

from test_ios_runtime_acceptance import CANDIDATE, exception_receipt, receipt, run_metadata


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ios-distribution.yml"
RECORDER = ROOT / ".github/workflows/ios-runtime-acceptance.yml"
POLICY_SHA = "9" * 40


def step(name, workflow=WORKFLOW):
    text = workflow.read_text().split("      - name: " + name + "\n", 1)[1]
    text = text.split("\n      - ", 1)[0]
    return textwrap.dedent(text.split("        run: |\n", 1)[1])


def cpu_only(proof):
    cpu = proof["quality"]["cpu"]
    del cpu["thermal_state"]
    cpu["method"] = "installed-foreground-cpu-v2"
    for window, percent in zip(cpu["windows"], (8, 2)):
        del window["disk_write_mib_per_minute"]
        window["cpu_percent_one_core"] = percent
    cpu["average_cpu_percent_one_core"] = 5
    return proof


@unittest.skipUnless(shutil.which("jq"), "The publisher requires jq")
class RuntimeWorkflowTests(unittest.TestCase):
    def test_missing_acceptance_fails_before_contact_and_internal_upload_remains_possible(self):
        script = step("Validate release tag")
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            gh = root / "gh"
            gh.write_text("#!" + sys.executable + "\n" + textwrap.dedent("""\
                import json, os, sys
                from pathlib import Path
                with Path(os.environ['CALLS']).open('a') as log:
                    log.write(json.dumps(sys.argv[1:]) + '\\n')
                if sys.argv[1:3] == ['release', 'view']:
                    print(json.dumps({'isDraft': False, 'isPrerelease': False,
                                      'isImmutable': True, 'url': 'https://example.invalid/release'}))
                """))
            gh.chmod(0o700)
            for target, run, ref, code in (
                ("testflight", "", "refs/tags/" + CANDIDATE["tag"], 0), ("testflight", "", "refs/heads/main", 2),
                ("testflight-public", "", "refs/heads/main", 2), ("app-store", "", "refs/heads/main", 2),
                ("app-store", "true", "refs/heads/main", 2), ("app-store", "123", "refs/heads/topic", 2),
                ("app-store", "123", "refs/tags/" + CANDIDATE["tag"], 2),
                ("testflight-public", "123", "refs/heads/main", 0), ("app-store", "123", "refs/heads/main", 0),
            ):
                with self.subTest(target=target, run=run, ref=ref):
                    calls = root / "calls"
                    calls.unlink(missing_ok=True)
                    env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                               GH_TOKEN="fixture", RELEASE_TAG=CANDIDATE["tag"],
                               GITHUB_REF=ref, DEFAULT_BRANCH="main",
                               DISTRIBUTION_TARGET=target, RUNTIME_ACCEPTANCE_RUN=run,
                               GITHUB_REPOSITORY="owner/project", GITHUB_OUTPUT=str(root / "output"),
                               CALLS=str(calls))
                    result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                            capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, code, result.stderr)
                    if code:
                        self.assertFalse(calls.exists(), "Missing proof must fail before external contact")

    def test_actual_promotion_step_rejects_wrong_proof_and_verifies_provenance(self):
        script = step("Require exact distributed-build runtime acceptance")
        for case in ("valid", "valid-adhoc", "wrong-source", "wrong-ipa", "failed-phase", "legacy", "warm-background-only",
                     "startup-crash", "restore-finished-before-background",
                     "adhoc-wrong-ipa", "adhoc-development", "adhoc-reset", "adhoc-no-proof",
                     "adhoc-missing-extension", "adhoc-mislabeled", "cpu-failure", "old-v2",
                     "startup-flash", "unreviewed-send",
                     "wrong-workflow", "failed-attestation", "missing-artifact", "historic-tag",
                     "wrong-tag-commit", "wrong-branch", "fork", "pull-request", "wrong-run",
                     "different-attempt", "different-signed-run", "tampered-source", "valid-cpu-only",
                     "documented-exception", "wrong-workflow-id", "wrong-workflow-resource-path",
                     "wrong-workflow-resource-name", "wrong-run-name", "wrong-display-title",
                     "canonical-name-with-title"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                (root / "source/scripts").mkdir(parents=True)
                (root / "policy/scripts").mkdir(parents=True)
                shutil.copy2(ROOT / "scripts/ios_runtime_acceptance.py",
                             root / "policy/scripts/ios_runtime_acceptance.py")
                # Exercise archived validator invocation, including its older CLI and schema.
                archived = root / "source/scripts/ios_runtime_acceptance.py"
                archived.write_text("import json, sys\nfrom pathlib import Path\n"
                    "a = sys.argv[1:]\n"
                    "assert '--recorder-commit' not in a and '--recorder-ref' not in a\n"
                    "proof = json.loads(Path(a[a.index('--receipt') + 1]).read_text())\n"
                    "assert proof == {'historic-tag-proof': a[a.index('--source-commit') + 1]}\n")
                proof, metadata = receipt(), run_metadata()
                candidate = CANDIDATE
                if case == "documented-exception":
                    proof = exception_receipt()
                    candidate = proof["release"]
                workflow = {"id": 391756692, "name": "iOS Runtime Acceptance",
                            "path": ".github/workflows/ios-runtime-acceptance.yml"}
                title = "Record iOS runtime acceptance " + candidate["tag"]
                metadata.update(id=123, run_attempt=2, workflow_id=workflow["id"], name=title,
                                display_title=title, head_sha=POLICY_SHA, head_branch="main",
                                repository={"full_name": "owner/project"})
                if case in ("historic-tag", "wrong-tag-commit"):
                    metadata.update(head_branch=CANDIDATE["tag"], head_sha=CANDIDATE["source_commit"])
                    proof = {"historic-tag-proof": CANDIDATE["source_commit"]}
                    metadata["name"] = workflow["name"]
                    del metadata["display_title"]
                if case == "canonical-name-with-title": metadata["name"] = workflow["name"]
                if case == "wrong-workflow-id": metadata["workflow_id"] += 1
                if case == "wrong-workflow-resource-path": workflow["path"] = ".github/workflows/ci.yml"
                if case == "wrong-workflow-resource-name": workflow["name"] = "iOS Distribution"
                if case == "wrong-run-name": metadata["name"] = "Record iOS runtime acceptance v2020.1.1"
                if case == "wrong-display-title":
                    metadata.update(name=workflow["name"], display_title="Record iOS runtime acceptance v2020.1.1")
                (root / "workflow.json").write_text(json.dumps(workflow))
                if case == "valid-cpu-only": proof = cpu_only(proof)
                if case == "wrong-tag-commit": metadata["head_sha"] = POLICY_SHA
                if case == "wrong-branch": metadata["head_branch"] = "topic"
                if case == "fork": metadata["head_repository"] = {"full_name": "other/project"}
                if case == "pull-request": metadata["event"] = "pull_request"
                if case == "wrong-run": metadata["id"] = 321
                if case == "tampered-source": proof["release"]["source_commit"] = POLICY_SHA
                if "adhoc" in case:
                    proof = receipt("authorized-production-adhoc")
                if case == "adhoc-wrong-ipa": proof["installation"]["production_adhoc"]["ipa_sha256"] = "0" * 64
                if case == "adhoc-development": proof["installation"]["production_adhoc"]["signing"] = "development"
                if case == "adhoc-reset": proof["installation"]["storage"] = "fresh-data"
                if case == "adhoc-no-proof": del proof["installation"]["production_adhoc"]
                if case == "adhoc-missing-extension": del proof["installation"]["production_adhoc"]["bundle_instruction_sha256"]["notification-service"]
                if case == "adhoc-mislabeled": proof["installation"]["channel"] = "testflight"
                if case == "cpu-failure": proof["quality"]["cpu"]["windows"][1]["cpu_percent_one_core"] = 5.3307874023075605
                if case == "old-v2":
                    proof.update(schema_version=2, contract="ios-distributed-upgrade-v2")
                    del proof["quality"]
                if case == "startup-flash": proof["quality"]["startup"]["login_flash_observed"] = True
                if case == "unreviewed-send": proof["quality"]["send"]["status"] = "pending-review"
                if case == "wrong-source": proof["release"]["source_commit"] = "0" * 40
                if case == "wrong-ipa": proof["release"]["ipa_sha256"] = "0" * 64
                if case == "failed-phase": proof["checks"]["background"]["status"] = "failed"
                if case == "legacy": proof = {"passed": True}
                if case == "warm-background-only":
                    proof.update(schema_version=1, contract="ios-distributed-upgrade-v1")
                    del proof["checks"]["cold_background_launch"]
                    del proof["checks"]["early_background"]
                if case == "startup-crash": proof["checks"]["cold_background_launch"]["unexpected_exits"] = 1
                if case == "restore-finished-before-background": proof["checks"]["early_background"]["restore_in_flight"] = False
                if case == "wrong-workflow": metadata["path"] = ".github/workflows/ci.yml"
                (root / "fixture.json").write_text(json.dumps(proof))
                (root / "metadata.json").write_text(json.dumps(metadata))
                gh = root / "gh"
                gh.write_text("#!" + sys.executable + "\n" + textwrap.dedent("""\
                    import json, os, shutil, sys
                    from pathlib import Path
                    args = sys.argv[1:]
                    root = Path(os.environ['FIXTURE_ROOT'])
                    with (root / 'calls').open('a') as log:
                        log.write(json.dumps(args) + '\\n')
                    if args[:2] == ['api', 'repos/owner/project/actions/runs/123']:
                        print((root / 'metadata.json').read_text())
                    elif args[:2] == ['api', 'repos/owner/project/actions/workflows/ios-runtime-acceptance.yml']:
                        print((root / 'workflow.json').read_text())
                    elif args[:2] == ['run', 'download']:
                        destination = Path(args[args.index('--dir') + 1])
                        destination.mkdir()
                        if os.environ['CASE'] != 'missing-artifact':
                            shutil.copy2(root / 'fixture.json', destination / 'ios-runtime-acceptance.json')
                    elif args[:2] == ['attestation', 'verify']:
                        if os.environ['CASE'] == 'failed-attestation':
                            sys.exit(37)
                        case = os.environ['CASE']
                        run = 321 if case == 'different-signed-run' else 123
                        attempt = 1 if case == 'different-attempt' else 2
                        uri = f'https://github.com/owner/project/actions/runs/{run}/attempts/{attempt}'
                        print(json.dumps([{'verificationResult': {'signature': {
                            'certificate': {'runInvocationURI': uri}}}}]))
                    else:
                        sys.exit('Unexpected external operation')
                    """))
                gh.chmod(0o700)
                env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                           FIXTURE_ROOT=str(root), CASE=case, GH_TOKEN="fixture", DEFAULT_BRANCH="main",
                           GITHUB_SERVER_URL="https://github.com",
                           RUNNER_TEMP=str(root), GITHUB_REPOSITORY="owner/project",
                           RELEASE_TAG=candidate["tag"], ACCEPTANCE_RUN="123",
                           SOURCE_COMMIT=candidate["source_commit"], IPA_SHA256=candidate["ipa_sha256"],
                           MANIFEST_SHA256=candidate["manifest_sha256"], APPLE_VERSION=candidate["apple_version"],
                           APPLE_BUILD=candidate["apple_build"])
                valid = case in ("valid", "valid-adhoc", "historic-tag", "valid-cpu-only",
                                 "documented-exception", "canonical-name-with-title")
                for target in (("app-store", "testflight-public") if valid else ("app-store",)):
                    with self.subTest(target=target):
                        # Each invocation gets a fresh hosted runner.
                        shutil.rmtree(root / "ios-runtime-acceptance", ignore_errors=True)
                        result = subprocess.run(["bash", "-c", script], cwd=root,
                                                env={**env, "DISTRIBUTION_TARGET": target},
                                                capture_output=True, text=True, timeout=10)
                        if case == "documented-exception" and target == "testflight-public":
                            self.assertNotEqual(result.returncode, 0, result.stdout)
                            self.assertIn("exception is authorized only for App Store submission", result.stdout)
                        elif valid:
                            self.assertEqual(result.returncode, 0, result.stderr)
                            calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
                            verify = next(args for args in calls if args[:2] == ["attestation", "verify"])
                            self.assertEqual(verify[verify.index("--source-ref") + 1],
                                             "refs/tags/" + candidate["tag"] if case == "historic-tag" else "refs/heads/main")
                            for flag in ("--source-digest", "--signer-digest"):
                                self.assertEqual(verify[verify.index(flag) + 1], metadata["head_sha"])
                            self.assertEqual(verify[verify.index("--signer-workflow") + 1],
                                             "owner/project/.github/workflows/ios-runtime-acceptance.yml")
                        else:
                            self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_recorder_binds_current_policy_to_the_original_candidate(self):
        script = step("Verify candidate and normalize safe acceptance record", RECORDER)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "source"
            source.mkdir()
            # A standalone candidate repository and a policy checkout with opposing validators.
            subprocess.run(["git", "init", "-q", str(source)], check=True)
            subprocess.run(["git", "-C", str(source), "-c", "user.name=Fixture", "-c",
                            "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                            "commit", "-qm", "candidate", "--allow-empty"], check=True)
            commit = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"],
                                             text=True).strip()
            (source / "scripts").mkdir()
            for filename in ("release-manifest.py", "build_common.sh"):
                shutil.copy2(ROOT / "scripts" / filename, source / "scripts" / filename)
            shutil.copy2(ROOT / "release-platforms.json", source)
            (source / "scripts/ios_runtime_acceptance.py").write_text("raise SystemExit(77)")
            (root / "scripts").mkdir()
            shutil.copy2(ROOT / "scripts/ios_runtime_acceptance.py", root / "scripts")
            tag = CANDIDATE["tag"]
            assets = root / "release"
            assets.mkdir()
            manifest_script = source / "scripts/release-manifest.py"
            names = subprocess.check_output([sys.executable, manifest_script, "list-assets",
                                              "--tag", tag], text=True).splitlines()
            for name in names:
                (assets / name).write_bytes(name.encode())
            manifest = assets / f"iris-chat-{tag}-manifest.json"
            subprocess.run([sys.executable, manifest_script, "create", "--tag", tag, "--commit", commit,
                            "--asset-dir", assets, "--out", manifest], check=True)
            ipa = assets / f"iris-chat-{tag}-ios.ipa"
            proof = cpu_only(receipt())
            proof["release"].update(source_commit=commit, ipa_sha256=hashlib.sha256(ipa.read_bytes()).hexdigest(),
                                    manifest_sha256=hashlib.sha256(manifest.read_bytes()).hexdigest())
            event = root / "event.json"
            event.write_text(json.dumps({"inputs": {"receipt_json": json.dumps(proof)}}))
            gh = root / "gh"
            gh.write_text("#!" + sys.executable + "\n" + textwrap.dedent("""\
                import os, sys, shutil
                from pathlib import Path
                args = sys.argv[1:]
                if args[:2] == ["release", "download"]:
                    for i, arg in enumerate(args):
                        if arg == "--pattern":
                            shutil.copyfile(Path("release") / args[i+1], Path(args[-1]) / args[i+1])
                elif args[:2] == ["attestation", "verify"]:
                    assert args[args.index("--source-digest")+1] == os.environ["CANDIDATE_SHA"]
                    assert args[args.index("--source-ref")+1] == "refs/tags/" + os.environ["RELEASE_TAG"]
                    assert args[args.index("--signer-workflow")+1] == "owner/project/.github/workflows/release.yml"
                elif args[:2] != ["release", "verify-asset"]:
                    raise SystemExit(9)
                """))
            gh.chmod(0o700)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                       RUNNER_TEMP=str(root), GITHUB_SHA=POLICY_SHA, GITHUB_REPOSITORY="owner/project",
                       GITHUB_EVENT_PATH=str(event), CANDIDATE_SHA=commit, RELEASE_TAG=tag)
            output = root / "acceptance/ios-runtime-acceptance.json"
            result = subprocess.run(["bash", "-c", script], env=env, cwd=root, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(output.read_text()), proof)
            output.unlink()
            ipa.write_bytes(b"modified fixture")
            bad = subprocess.run(["bash", "-c", script], env=env, cwd=root, text=True, capture_output=True)
            self.assertNotEqual(bad.returncode, 0)
            self.assertFalse(output.exists())

    def test_promotion_is_gated_before_apple_credentials_and_recorder_is_tag_bound(self):
        workflow = WORKFLOW.read_text()
        start = workflow.index("      - name: Require exact distributed-build runtime acceptance\n")
        self.assertLess(start, workflow.index("      - name: Configure App Store Connect authentication\n"))
        acceptance_step = workflow[start:workflow.index("      - name: Prepare Apple release notes", start)]
        self.assertIn("        if: inputs.target != 'testflight'\n", acceptance_step)
        self.assertIn("DISTRIBUTION_TARGET: ${{ inputs.target }}", acceptance_step)
        producer = (ROOT / ".github/workflows/ios-runtime-acceptance.yml").read_text()
        self.assertIn('"$GITHUB_REF" != "refs/heads/$DEFAULT_BRANCH"', producer)
        self.assertIn('ref: ${{ github.sha }}', producer)
        self.assertIn('ref: ${{ github.sha }}', workflow)
        self.assertIn('ref: refs/tags/${{ steps.release.outputs.tag }}', workflow)
        self.assertIn('--source-commit "$candidate_commit"', producer)
        self.assertIn('--source-digest "$candidate_commit"', producer)
        self.assertIn("environment: ios-app-store-release", producer)
        self.assertLess(producer.index("python3 scripts/ios_runtime_acceptance.py"),
                        producer.index("uses: actions/attest@"))
        self.assertIn("subject-path: acceptance/ios-runtime-acceptance.json", producer)
        self.assertIn("if-no-files-found: error", producer)
        self.assertNotIn("ASC_PRIVATE_KEY", producer)


if __name__ == "__main__":
    unittest.main()
