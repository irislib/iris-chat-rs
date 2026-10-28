"""Script-only contracts for trusted main CI to seed tagged release caches."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


def job(text, name, next_name):
    return text.split(f"\n  {name}:\n", 1)[1].split(f"\n  {next_name}:\n", 1)[0]


def step(text, name):
    return text.split(f"      - name: {name}\n", 1)[1].split("\n      - name:", 1)[0]


def evaluate(expression, values):
    for field, value in values.items():
        expression = expression.replace(field, repr(value))
    expression = re.sub(r"!(?!=)", "not ", expression).replace("&&", "and").replace("||", "or")
    return eval(expression, {"__builtins__": {},
                             "endsWith": lambda value, suffix: value.endswith(suffix),
                             "format": lambda pattern, value: pattern.format(value)})


class MainReleaseCacheTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ci = job((ROOT / ".github/workflows/ci.yml").read_text(), "rust", "android")
        cls.release = job((ROOT / ".github/workflows/release.yml").read_text(), "verify", "mesh-resource")

    def test_only_trusted_full_main_runs_seed_release_cache(self):
        expression = re.search(r"RELEASE_CACHE_TRUSTED: \$\{\{ (.+) \}\}", self.ci).group(1)
        for event, ref, scope, expected in [
            ("push", "refs/heads/main", "", True),
            ("workflow_dispatch", "refs/heads/main", "core", True),
            ("workflow_dispatch", "refs/heads/main", "all", True),
            ("workflow_dispatch", "refs/heads/main", "protocol", False),
            ("push", "refs/heads/dev", "", False),
            ("pull_request", "refs/pull/42/merge", "", False),
            ("pull_request_target", "refs/heads/main", "", False),
            ("workflow_run", "refs/heads/main", "", False),
        ]:
            with self.subTest(event=event, ref=ref, scope=scope):
                self.assertEqual(evaluate(expression, {"github.event_name": event,
                    "github.ref": ref, "inputs.scope": scope}), expected)
        for name in ("Install system dependencies", "Install Rust", "Cache Cargo"):
            self.assertIn("if: env.RELEASE_CACHE_TRUSTED != 'true'", step(self.ci, name))
        legacy = step(self.ci, "Cache Cargo")
        self.assertIn("key: rust-${{ runner.os }}-", legacy)
        self.assertNotIn("rust-release-v2", legacy)
        for name in ("Install checksum-verified release compiler", "Install native dependencies and identify compiler", "Restore release compiler cache"):
            self.assertIn("if: env.RELEASE_CACHE_TRUSTED == 'true'", step(self.ci, name))

    def test_release_inputs_paths_and_lock_bound_keys_match(self):
        for name in ("Install checksum-verified release compiler", "Install native dependencies and identify compiler"):
            self.assertEqual(step(self.ci, name).split("        run: ", 1)[1],
                             step(self.release, name).split("        run: ", 1)[1])
        restore = step(self.ci, "Restore release compiler cache")
        release = step(self.release, "Restore release compiler cache")
        key = lambda text: next(line.strip() for line in text.splitlines() if line.strip().startswith("key: "))
        self.assertEqual(key(restore), key(release).replace("needs.validate.outputs.sha", "github.sha"))
        paths = lambda text: text.split("          path: |\n", 1)[1].split("          key:", 1)[0]
        save = step(self.ci, "Preserve release compiler cache after tests")
        self.assertEqual(paths(restore), paths(release)); self.assertEqual(paths(save), paths(release))
        self.assertIn("CARGO_TARGET_DIR: ${{ github.workspace }}/cargo-target", self.ci)
        self.assertEqual(re.search(r"uses: (actions/cache/restore@\S+)", restore).group(1),
                         re.search(r"uses: (actions/cache/restore@\S+)", release).group(1))
        for prefix in filter(str.strip, restore.split("restore-keys: |\n", 1)[1].splitlines()):
            self.assertIn("steps.toolchain.outputs.identity", prefix)
            self.assertIn("release-build-inputs.json", prefix)
            self.assertIn("apt-snapshots.tsv", prefix)
            self.assertIn("hashFiles('iris-chat-rs/**/Cargo.lock')", prefix)
        for text in (restore, save):
            self.assertNotIn("github.run_id", text); self.assertNotIn("github.run_attempt", text)
        self.assertRegex(save, r"uses: actions/cache/save@[0-9a-f]{40} # v6")

    def test_partial_failure_completion_and_no_repeated_partial_save(self):
        save = step(self.ci, "Preserve release compiler cache after tests")
        expression = re.search(r"if: \$\{\{ (.+) \}\}", save).group(1)
        self.assertIn("${{ job.status == 'success' && 'complete' || 'partial' }}", save)
        self.assertLess(self.ci.index("Upload verified protocol package"), self.ci.index("Preserve release compiler cache after tests"))
        partial = "compiler-inputs-lock-" + "a" * 40 + "-partial"
        for name, trusted, cancelled, restored, status, hit, matched, expected in [
            ("cold pass", "true", False, "success", "success", "false", "", True),
            ("cold failure", "true", False, "success", "failure", "false", "", True),
            ("complete hit", "true", False, "success", "success", "true", "complete", False),
            ("partial completed", "true", False, "success", "success", "false", partial, True),
            ("partial failed again", "true", False, "success", "failure", "false", partial, False),
            ("cancelled", "true", True, "success", "cancelled", "false", "", False),
            ("restore failed", "true", False, "failure", "success", "false", "", False),
            ("PR", "false", False, "success", "success", "false", "", False),
        ]:
            with self.subTest(case=name):
                self.assertEqual(evaluate(expression, {"env.RELEASE_CACHE_TRUSTED": trusted,
                    "cancelled()": cancelled, "steps.release-cargo-cache.outcome": restored,
                    "steps.release-cargo-cache.outputs.cache-hit": hit,
                    "steps.release-cargo-cache.outputs.cache-matched-key": matched,
                    "job.status": status, "github.sha": "a" * 40}), expected)
        # Cache hits never skip the existing lint, native fixture, CPU/privacy or package checks.
        for name in ("Static checks", "Rust formatting", "Rust lint including test targets", "Rust tests", "Protocol tests", "Verify protocol package"):
            body = step(self.ci, name)
            self.assertNotIn("cache-hit", body); self.assertNotIn("RELEASE_CACHE_TRUSTED", body)
        self.assertIn('./scripts/test_rust.sh\n', step(self.ci, 'Rust tests'))
        self.assertNotIn("continue-on-error:", self.ci)

    def test_main_compiles_the_same_expensive_release_profiles(self):
        fixture = 'IRIS_PRIVATE_CONTROL_FIXTURE_OUTPUT="$PWD/work/private-controls/native-private-controls.json"'
        self.assertIn(fixture + ' ./scripts/test_rust.sh', step(self.ci, 'Rust tests'))
        self.assertIn(fixture + ' scripts/verify.sh fast', step(self.release, 'Run hosted release gate'))
        verify = (ROOT / 'scripts/verify.sh').read_text()
        fast = (ROOT / 'scripts/test_fast.sh').read_text()
        self.assertIn('scripts/test_fast.sh --core-only', verify)
        self.assertIn('run_step "Rust tests" "${ROOT_DIR}/scripts/test_rust.sh"', fast)
        runner = (ROOT / 'scripts/test_rust.sh').read_text()
        self.assertIn('crates=(core chat-protocol protocol-ffi)', runner)
        self.assertIn('args=(--manifest-path "${ROOT_DIR}/${crate}/Cargo.toml" --locked)', runner)
        for test in ('populated_account_five_relay_discovery_stays_quiet_and_targeted_demand_progresses',
                     'restored_backlog_batched_retry_stays_within_cpu_budget'):
            command = runner.split(test, 1)[0].rsplit('CARGO_PROFILE_RELEASE_OPT_LEVEL=3', 1)[1]
            self.assertIn('CARGO_PROFILE_RELEASE_DEBUG=1', command)
            self.assertIn('CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false', command)
            self.assertIn('cargo test "${args[@]}" --release --jobs 2 --lib', command)

    def test_existing_core_dispatch_selects_no_unrelated_native_jobs(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text().split('\njobs:\n', 1)[1]
        selected = []
        for name, expression in re.findall(r'^  ([\w-]+):\n    if: \$\{\{ (.+) \}\}', workflow, re.MULTILINE):
            if evaluate(expression, {'inputs.scope': 'core'}):
                selected.append(name)
        self.assertEqual(selected, ['rust'])


if __name__ == "__main__":
    unittest.main()
