#!/usr/bin/env python3
"""Contract checks only: no phones, Apple credentials, or builds."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import ios_runtime_acceptance as acceptance


CANDIDATE = {
    "tag": "v2026.10.21.1", "source_commit": "a" * 40,
    "ipa_sha256": "b" * 64, "manifest_sha256": "c" * 64,
    "apple_version": "2026.10.2101", "apple_build": "2026102101",
}


def receipt(channel="testflight"):
    phase = {"status": "passed", "observed_seconds": 60, "process_samples": 2,
             "unexpected_exits": 0, "process_replacements": 0,
             "process_state": "foreground", "evidence_sha256": "d" * 64}
    phases = {name: copy.deepcopy(phase)
              for name in ("foreground", "background", "resume", "cold_relaunch", "affected_flow",
                           "cold_background_launch", "early_background")}
    phases["background"].update(observed_seconds=135, process_state="background-suspended")
    phases["cold_background_launch"].update(
        observed_seconds=135, process_state="background-suspended", entry_state="background",
        foreground_activations=0)
    phases["early_background"].update(
        observed_seconds=135, process_state="background-suspended", restore_in_flight=True)
    for name in ("resume", "cold_relaunch", "affected_flow"):
        phases[name]["observed_seconds"] = 1
    result = {
        "schema_version": 3, "contract": "ios-release-upgrade-v3", "release": dict(CANDIDATE),
        "installation": {
            "channel": "testflight", "device": "physical-iphone", "previous_build": "2026101800",
            "installed_build": CANDIDATE["apple_build"], "storage": "preserved-app-data-and-keychain",
            "account": "preserved", "history": "preserved", "draft": "preserved", "evidence_sha256": "e" * 64,
        },
        "checks": phases,
        "crash_check": {"status": "passed", "unexpected_reports": 0,
                        "unexpected_exits": 0, "evidence_sha256": "f" * 64},
        "quality": {
            "cpu": {"build": CANDIDATE["apple_build"], "status": "passed",
                    "method": "installed-foreground-cpu-v1", "evidence_sha256": "1" * 64,
                    "qualification_sha256": "2" * 64, "warmup_seconds": 180,
                    "discarded_interval_seconds": 60, "foreground_sample_max_gap_seconds": 1.1,
                    "process_sample_max_gap_seconds": 5.5, "invalid_observations": 0,
                    "process_replacements": 0, "thermal_state": "nominal",
                    "windows": [{"observed_seconds": 60, "metric_seconds": 59,
                                 "cpu_percent_one_core": 4, "disk_write_mib_per_minute": 0.2}
                                for _ in range(2)]},
            "startup": {"build": CANDIDATE["apple_build"], "status": "accepted-after-review",
                        "flow": "cold-start-to-preserved-state", "login_flash_observed": False,
                        "observations_sha256": "3" * 64, "regression_sha256": "4" * 64,
                        "review_sha256": "5" * 64},
            "send": {"build": CANDIDATE["apple_build"], "status": "accepted-after-review",
                     "send_to_visible_seconds": 0.3, "observations_sha256": "6" * 64,
                     "review_sha256": "7" * 64},
        },
    }
    result["installation"]["channel"] = channel
    if channel == acceptance.ADHOC_CHANNEL:
        result["installation"]["production_adhoc"] = {
            **{field: "8" * 64 for field in ("authorization_sha256", "release_acceptance_sha256",
                "preparation_sha256", "installation_sha256", "original_signing_sha256",
                "profiles_sha256", "normal_launch_sha256", "qualification_sha256")},
            "authorization": "explicit-user-direct-install", "normal_launch_route": "accepted-adhoc-normal-launch",
            "code_provenance": "official-tagged-ipa-instructions-unchanged", "signing": "production-adhoc",
            "push_environment": "production", "debugging": "disabled", "original_ipa": "unchanged",
            "entitlement_delta": "beta-reports-active-removal-only", "state_policy": "in-place-no-reset-or-restore",
            "ipa_sha256": CANDIDATE["ipa_sha256"],
            "bundle_instruction_sha256": {role: "9" * 64 for role in
                                          ("app", "notification-service", "share-extension")},
            "testflight_upgrade_claimed": False, "installed_binary_extraction_claimed": False,
        }
    return result


def run_metadata():
    return {"status": "completed", "conclusion": "success", "event": "workflow_dispatch",
            "head_sha": CANDIDATE["source_commit"], "head_repository": {"full_name": "owner/project"},
            "path": ".github/workflows/ios-runtime-acceptance.yml", "name": "iOS Runtime Acceptance"}


class RuntimeAcceptanceTests(unittest.TestCase):
    def validate(self, value):
        return acceptance.validate_receipt(value, **CANDIDATE)

    def test_exact_candidate_normalizes_without_expiry_or_mutating_input(self):
        value = receipt()
        original = copy.deepcopy(value)
        self.assertEqual(self.validate(value), original)
        self.assertEqual(value, original)
        # The receipt has no clock/expiry dependency; unchanged candidate proof is reusable.
        self.assertEqual(self.validate(value), self.validate(value))

    def test_historical_exact_candidate_has_no_arbitrary_expiry(self):
        candidate = {**CANDIDATE, "tag": "v2024.1.1.1", "apple_version": "2024.1.101", "apple_build": "2024010101"}
        value = receipt(); value["release"] = candidate.copy()
        value["installation"].update(previous_build="2023123100", installed_build=candidate["apple_build"])
        for check in value["quality"].values():
            check["build"] = candidate["apple_build"]
        self.assertEqual(acceptance.validate_receipt(value, **candidate), value)

    def test_each_release_binding_is_required(self):
        alternatives = {"tag": "v2026.10.21.2", "source_commit": "0" * 40,
                        "ipa_sha256": "0" * 64, "manifest_sha256": "0" * 64,
                        "apple_version": "2026.10.2102", "apple_build": "2026102102"}
        for field, wrong in alternatives.items():
            with self.subTest(field=field):
                value = receipt(); value["release"][field] = wrong
                with self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_authorized_production_adhoc_requires_every_proof_and_bundle(self):
        value = receipt(acceptance.ADHOC_CHANNEL)
        self.assertEqual(self.validate(value), value)
        proof = value["installation"]["production_adhoc"]
        for field in proof:
            invalid = copy.deepcopy(value); del invalid["installation"]["production_adhoc"][field]
            with self.subTest(missing=field), self.assertRaises(acceptance.ValidationError): self.validate(invalid)
        alternatives = {**{field: "/private/evidence" for field in acceptance.ADHOC_DIGESTS},
                        **{field: "unverified" for field in acceptance.ADHOC_POLICY},
                        "ipa_sha256": "0" * 64, "testflight_upgrade_claimed": True,
                        "installed_binary_extraction_claimed": True}
        for field, wrong in alternatives.items():
            invalid = copy.deepcopy(value); invalid["installation"]["production_adhoc"][field] = wrong
            with self.subTest(field=field), self.assertRaises(acceptance.ValidationError): self.validate(invalid)
        for role in proof["bundle_instruction_sha256"]:
            invalid = copy.deepcopy(value); del invalid["installation"]["production_adhoc"]["bundle_instruction_sha256"][role]
            with self.subTest(bundle=role), self.assertRaises(acceptance.ValidationError): self.validate(invalid)
        for path in (("production_adhoc",), ("production_adhoc", "bundle_instruction_sha256")):
            invalid = copy.deepcopy(value); target = invalid["installation"]
            for part in path: target = target[part]
            target["private_path"] = "never-copy"
            with self.subTest(path=path), self.assertRaises(acceptance.ValidationError): self.validate(invalid)
        for field in ("testflight_upgrade_claimed", "installed_binary_extraction_claimed"):
            invalid = copy.deepcopy(value); invalid["installation"]["production_adhoc"][field] = 0
            with self.subTest(boolean=field), self.assertRaises(acceptance.ValidationError): self.validate(invalid)

    def test_development_or_mislabeled_installation_cannot_reuse_adhoc_proof(self):
        for channel in ("testflight", "development", "adhoc", "authorized-production-adhoc-unverified"):
            value = receipt(acceptance.ADHOC_CHANNEL); value["installation"]["channel"] = channel
            with self.subTest(channel=channel), self.assertRaises(acceptance.ValidationError): self.validate(value)
        value = receipt(); value["installation"]["channel"] = acceptance.ADHOC_CHANNEL
        with self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_both_routes_require_cpu_windows_without_averaging_away_failure(self):
        for channel in ("testflight", acceptance.ADHOC_CHANNEL):
            valid = receipt(channel)
            for key, wrong in (("cpu_percent_one_core", 5.3307874023075605),
                               ("disk_write_mib_per_minute", 5.01), ("observed_seconds", 59.8),
                               ("metric_seconds", 56.9), ("metric_seconds", 57),
                               ("cpu_percent_one_core", True), ("cpu_percent_one_core", float("nan")),
                               ("disk_write_mib_per_minute", -0.1)):
                value = copy.deepcopy(valid); value["quality"]["cpu"]["windows"][1][key] = wrong
                with self.subTest(channel=channel, key=key, wrong=wrong), self.assertRaises(acceptance.ValidationError):
                    self.validate(value)
            for field, wrong in (("build", "2026101800"), ("status", "failed"), ("method", "simulated"),
                                 ("thermal_state", "fair"), ("warmup_seconds", 179),
                                 ("discarded_interval_seconds", 59), ("foreground_sample_max_gap_seconds", 2.6),
                                 ("process_sample_max_gap_seconds", 8.1), ("invalid_observations", 1),
                                 ("process_replacements", 1)):
                value = copy.deepcopy(valid); value["quality"]["cpu"][field] = wrong
                with self.subTest(channel=channel, field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)
            for windows in ([], valid["quality"]["cpu"]["windows"][:1], valid["quality"]["cpu"]["windows"] * 2):
                value = copy.deepcopy(valid); value["quality"]["cpu"]["windows"] = windows
                with self.subTest(channel=channel, count=len(windows)), self.assertRaises(acceptance.ValidationError): self.validate(value)
        value = receipt()
        for window in value["quality"]["cpu"]["windows"]:
            window.update(cpu_percent_one_core=5, disk_write_mib_per_minute=5)
        self.assertEqual(self.validate(value), value)

    def test_startup_and_send_require_candidate_bound_observations_and_review(self):
        for name, field, wrong in (("startup", "login_flash_observed", True),
                                   ("startup", "login_flash_observed", 0), ("startup", "flow", "already-running"),
                                   ("send", "send_to_visible_seconds", -1),
                                   ("send", "send_to_visible_seconds", float("inf")),
                                   ("send", "send_to_visible_seconds", True)):
            value = receipt(); value["quality"][name][field] = wrong
            with self.subTest(name=name, field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)
        for name in ("startup", "send"):
            for field, wrong in (("build", "2026101800"), ("status", "pending-review"),
                                 ("observations_sha256", ""), ("review_sha256", "")):
                value = receipt(); value["quality"][name][field] = wrong
                with self.subTest(name=name, field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)
        # Observed latency requires review, without fabricating a new numeric budget.
        value = receipt(); value["quality"]["send"]["send_to_visible_seconds"] = 3
        self.assertEqual(self.validate(value), value)

    def test_v2_cannot_bypass_new_quality_gates(self):
        value = receipt(); value.update(schema_version=2, contract="ios-distributed-upgrade-v2")
        del value["quality"]
        with self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_upgrade_must_preserve_populated_physical_testflight_account(self):
        alternatives = {"channel": "development", "device": "simulator", "previous_build": "2026102101",
                        "installed_build": "2026102102", "storage": "fresh-data", "account": "reset",
                        "history": "empty", "draft": "lost", "evidence_sha256": "x" * 64}
        for field, wrong in alternatives.items():
            with self.subTest(field=field):
                value = receipt(); value["installation"][field] = wrong
                with self.assertRaises(acceptance.ValidationError): self.validate(value)
        for wrong in (True, 2026101800, "", "-1", "2026102200"):
            value = receipt(); value["installation"]["previous_build"] = wrong
            with self.subTest(previous_build=wrong), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_every_phase_and_exact_process_state_is_mandatory(self):
        for phase in receipt()["checks"]:
            for field, wrong in (("status", "failed"), ("observed_seconds", 0), ("process_samples", 1),
                                 ("unexpected_exits", 1), ("process_replacements", 1),
                                 ("process_state", "background"), ("evidence_sha256", "D" * 64)):
                value = receipt(); value["checks"][phase][field] = wrong
                with self.subTest(phase=phase, field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)
            value = receipt(); del value["checks"][phase]
            with self.subTest(missing=phase), self.assertRaises(acceptance.ValidationError): self.validate(value)
        for phase, duration in (("foreground", 59.999), ("background", 134.999),
                                ("cold_background_launch", 134.999), ("early_background", 134.999)):
            value = receipt(); value["checks"][phase]["observed_seconds"] = duration
            with self.subTest(duration=duration), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_warm_background_or_foreground_launch_cannot_replace_startup_coverage(self):
        cases = (("cold_background_launch", "entry_state", "foreground"),
                 ("cold_background_launch", "foreground_activations", 1),
                 ("cold_background_launch", "foreground_activations", False),
                 ("early_background", "restore_in_flight", False),
                 ("early_background", "restore_in_flight", 1))
        for phase, field, wrong in cases:
            value = receipt(); value["checks"][phase][field] = wrong
            with self.subTest(phase=phase, field=field, wrong=wrong), self.assertRaises(acceptance.ValidationError):
                self.validate(value)
        for phase in ("cold_background_launch", "early_background"):
            value = receipt(); value["checks"][phase] = copy.deepcopy(value["checks"]["background"])
            with self.subTest(warm_copy=phase), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_legacy_receipt_cannot_skip_new_background_startup_coverage(self):
        value = receipt()
        value.update(schema_version=1, contract="ios-distributed-upgrade-v1")
        del value["checks"]["cold_background_launch"]
        del value["checks"]["early_background"]
        with self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_reported_background_flow_requires_actual_suspension_survival(self):
        value = receipt()
        phase = value["checks"]["affected_flow"]
        phase.update(process_state="background-suspended", observed_seconds=135)
        self.assertEqual(self.validate(value), value)
        for field, wrong in (("observed_seconds", 134.999), ("process_state", "background"),
                             ("unexpected_exits", 1), ("process_replacements", 1)):
            invalid = copy.deepcopy(value); invalid["checks"]["affected_flow"][field] = wrong
            with self.subTest(field=field), self.assertRaises(acceptance.ValidationError): self.validate(invalid)

    def test_booleans_nonfinite_and_noninteger_counts_are_not_proof(self):
        for field in ("observed_seconds", "process_samples", "unexpected_exits", "process_replacements"):
            for wrong in (True, False, float("nan"), float("inf"), -float("inf"), "60", None):
                value = receipt(); value["checks"]["foreground"][field] = wrong
                with self.subTest(field=field, wrong=wrong), self.assertRaises(acceptance.ValidationError): self.validate(value)
        for field, wrong in (("process_samples", 2.0), ("unexpected_exits", 0.0), ("process_replacements", 0.0)):
            value = receipt(); value["checks"]["foreground"][field] = wrong
            with self.subTest(field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_unknown_fields_are_rejected_at_every_receipt_level(self):
        paths = [(), ("release",), ("installation",), ("checks",), ("crash_check",), ("quality",)]
        paths += [("checks", name) for name in receipt()["checks"]]
        paths += [("quality", name) for name in receipt()["quality"]]
        paths += [("quality", "cpu", "windows", 0)]
        for path in paths:
            value = receipt(); target = value
            for part in path: target = target[part]
            target["private_device_account_path_or_secret"] = "must not be copied"
            with self.subTest(path=path), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_incomplete_legacy_or_naked_success_and_crash_reports_are_rejected(self):
        for invalid in (True, {"passed": True}, {"contract": "same-executable-pid-15s-v1"}, [], None):
            with self.subTest(invalid=invalid), self.assertRaises(acceptance.ValidationError): self.validate(invalid)
        for field, wrong in (("status", "failed"), ("unexpected_reports", 1), ("unexpected_exits", True),
                             ("evidence_sha256", "")):
            value = receipt(); value["crash_check"][field] = wrong
            with self.subTest(field=field), self.assertRaises(acceptance.ValidationError): self.validate(value)

    def test_run_must_be_successful_owned_exact_workflow_source_and_dispatch(self):
        expected = run_metadata()
        self.assertEqual(acceptance.validate_run(expected, "owner/project", CANDIDATE["source_commit"]), expected)
        alternatives = {"status": "in_progress", "conclusion": "failure", "event": "push",
                        "head_sha": "0" * 40, "head_repository": {"full_name": "attacker/project"},
                        "path": ".github/workflows/ci.yml", "name": "Native CI"}
        for field, wrong in alternatives.items():
            value = run_metadata(); value[field] = wrong
            with self.subTest(field=field), self.assertRaises(acceptance.ValidationError):
                acceptance.validate_run(value, "owner/project", CANDIDATE["source_commit"])

    def test_real_api_metadata_extras_are_not_copied_to_normalized_provenance(self):
        value = run_metadata()
        value["path"] += "@refs/heads/main"
        value["unrelated_api_field"] = {"private_capture_path": "never-copy"}
        value["head_repository"]["unrelated_api_field"] = "never-copy"
        normalized = acceptance.validate_run(value, "owner/project", CANDIDATE["source_commit"])
        self.assertEqual(normalized, run_metadata())
        self.assertNotIn("never-copy", json.dumps(normalized))
        for path in (".github/workflows/ios-runtime-acceptance.yml@", ".github/workflows/ios-runtime-acceptance.yml.bak",
                     ".github/workflows/ios-runtime-acceptance.yml@refs/heads/main@other"):
            value["path"] = path
            with self.subTest(path=path), self.assertRaises(acceptance.ValidationError):
                acceptance.validate_run(value, "owner/project", CANDIDATE["source_commit"])

    def test_cli_only_outputs_normalized_whitelisted_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "input.json"; path.write_text(json.dumps(receipt()))
            command = [sys.executable, "-B", str(Path(acceptance.__file__)), "--receipt", str(path)]
            for field, value in CANDIDATE.items(): command += ["--" + field.replace("_", "-"), value]
            output = Path(temp) / "normalized.json"
            result = subprocess.run(command + ["--out", str(output)], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), receipt())
            self.assertEqual(json.loads(output.read_text()), receipt())
            value = receipt(); value["private_field"] = "/private/fixture/never-echo-this"
            path.write_text(json.dumps(value))
            result = subprocess.run(command, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertNotIn("never-echo", result.stderr)
            self.assertNotIn(str(path), result.stderr)

    def test_cli_rejects_duplicate_nonfinite_and_invalid_json_without_echoing_it(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "private-input.json"
            command = [sys.executable, "-B", str(Path(acceptance.__file__)), "--receipt", str(path)]
            for field, value in CANDIDATE.items(): command += ["--" + field.replace("_", "-"), value]
            for raw in ('{"schema_version":1,"schema_version":1}', '{"value": NaN}', '{"value": Infinity}',
                        '{"private_path":"never-echo-this",', 'true', '\ufffd'):
                path.write_text(raw)
                result = subprocess.run(command, text=True, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                self.assertEqual(result.stderr, "Invalid iOS runtime acceptance input.\n")

    def test_cli_binds_optional_run_metadata_and_requires_both_arguments(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "input.json"; path.write_text(json.dumps(receipt()))
            run = Path(temp) / "run.json"; run.write_text(json.dumps(run_metadata()))
            command = [sys.executable, "-B", str(Path(acceptance.__file__)), "--receipt", str(path)]
            for field, value in CANDIDATE.items(): command += ["--" + field.replace("_", "-"), value]
            valid = subprocess.run(command + ["--run-metadata", str(run), "--repository", "owner/project"],
                                   text=True, capture_output=True)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertEqual(json.loads(valid.stdout), receipt())
            for extra in (["--run-metadata", str(run)], ["--repository", "owner/project"],
                          ["--run-metadata", str(run), "--repository", "other/project"]):
                result = subprocess.run(command + extra, text=True, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
