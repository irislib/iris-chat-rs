#!/usr/bin/env python3
"""Exercise the real promotion shell with GitHub replaced at its boundary."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

from test_ios_runtime_acceptance import CANDIDATE, receipt, run_metadata


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ios-distribution.yml"


def step(name):
    text = WORKFLOW.read_text().split("      - name: " + name + "\n", 1)[1]
    text = text.split("\n      - name:", 1)[0]
    return textwrap.dedent(text.split("        run: |\n", 1)[1])


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
            for target, run, code in (
                ("testflight", "", 0), ("testflight-public", "", 2),
                ("app-store", "", 2), ("app-store", "true", 2),
                ("testflight-public", "123", 0), ("app-store", "123", 0),
            ):
                with self.subTest(target=target, run=run):
                    calls = root / "calls"
                    calls.unlink(missing_ok=True)
                    env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                               GH_TOKEN="fixture", RELEASE_TAG=CANDIDATE["tag"],
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
        for case in ("valid", "wrong-source", "wrong-ipa", "failed-phase", "legacy", "warm-background-only",
                     "startup-crash", "restore-finished-before-background",
                     "wrong-workflow", "failed-attestation", "missing-artifact"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                (root / "source/scripts").mkdir(parents=True)
                shutil.copy2(ROOT / "scripts/ios_runtime_acceptance.py",
                             root / "source/scripts/ios_runtime_acceptance.py")
                proof, metadata = receipt(), run_metadata()
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
                    if args[0] == 'api':
                        print((root / 'metadata.json').read_text())
                    elif args[:2] == ['run', 'download']:
                        destination = Path(args[args.index('--dir') + 1])
                        destination.mkdir()
                        if os.environ['CASE'] != 'missing-artifact':
                            shutil.copy2(root / 'fixture.json', destination / 'ios-runtime-acceptance.json')
                    elif args[:2] == ['attestation', 'verify']:
                        if os.environ['CASE'] == 'failed-attestation':
                            sys.exit(37)
                    else:
                        sys.exit('Unexpected external operation')
                    """))
                gh.chmod(0o700)
                env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                           FIXTURE_ROOT=str(root), CASE=case, GH_TOKEN="fixture",
                           RUNNER_TEMP=str(root), GITHUB_REPOSITORY="owner/project",
                           RELEASE_TAG=CANDIDATE["tag"], ACCEPTANCE_RUN="123",
                           SOURCE_COMMIT=CANDIDATE["source_commit"],
                           IPA_SHA256=CANDIDATE["ipa_sha256"], MANIFEST_SHA256=CANDIDATE["manifest_sha256"],
                           APPLE_VERSION=CANDIDATE["apple_version"], APPLE_BUILD=CANDIDATE["apple_build"])
                result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                        capture_output=True, text=True, timeout=10)
                if case == "valid":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()]
                    verify = next(args for args in calls if args[:2] == ["attestation", "verify"])
                    self.assertEqual(verify[verify.index("--source-ref") + 1],
                                     "refs/tags/" + CANDIDATE["tag"])
                    self.assertEqual(verify[verify.index("--source-digest") + 1], CANDIDATE["source_commit"])
                    self.assertEqual(verify[verify.index("--signer-workflow") + 1],
                                     "owner/project/.github/workflows/ios-runtime-acceptance.yml")
                else:
                    self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_promotion_is_gated_before_apple_credentials_and_recorder_is_tag_bound(self):
        workflow = WORKFLOW.read_text()
        start = workflow.index("      - name: Require exact distributed-build runtime acceptance\n")
        self.assertLess(start, workflow.index("      - name: Configure App Store Connect authentication\n"))
        self.assertIn("        if: inputs.target != 'testflight'\n", workflow[start:])
        producer = (ROOT / ".github/workflows/ios-runtime-acceptance.yml").read_text()
        self.assertIn('"$GITHUB_REF" != "refs/tags/$RELEASE_TAG"', producer)
        self.assertIn("environment: ios-app-store-release", producer)
        self.assertLess(producer.index("python3 scripts/ios_runtime_acceptance.py"),
                        producer.index("uses: actions/attest@v4"))
        self.assertIn("subject-path: acceptance/ios-runtime-acceptance.json", producer)
        self.assertIn("if-no-files-found: error", producer)
        self.assertNotIn("ASC_PRIVATE_KEY", producer)


if __name__ == "__main__":
    unittest.main()
