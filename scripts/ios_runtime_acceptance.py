#!/usr/bin/env python3
"""Validate a safe, candidate-bound receipt for a distributed iOS upgrade."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys


CONTRACT = "ios-release-upgrade-v3"
WORKFLOW = ".github/workflows/ios-runtime-acceptance.yml"
WORKFLOW_NAME = "iOS Runtime Acceptance"
RELEASE_KEYS = {"tag", "source_commit", "ipa_sha256", "manifest_sha256", "apple_version", "apple_build"}
PHASES = {"foreground": (60, "foreground"), "background": (135, "background-suspended"),
          "resume": (1, "foreground"), "cold_relaunch": (1, "foreground"), "affected_flow": (1, "foreground"),
          "cold_background_launch": (135, "background-suspended"),
          "early_background": (135, "background-suspended")}
PHASE_KEYS = {"status", "observed_seconds", "process_samples", "unexpected_exits",
              "process_replacements", "process_state", "evidence_sha256"}
STARTUP_PHASE_KEYS = {"cold_background_launch": {"entry_state", "foreground_activations"},
                      "early_background": {"restore_in_flight"}}
MAX_JSON_BYTES = 256 * 1024
ADHOC_CHANNEL = "authorized-production-adhoc"
ADHOC_DIGESTS = {"authorization_sha256", "release_acceptance_sha256", "preparation_sha256",
                 "installation_sha256", "original_signing_sha256", "profiles_sha256",
                 "normal_launch_sha256", "qualification_sha256"}
ADHOC_POLICY = {"authorization": "explicit-user-direct-install",
                "normal_launch_route": "accepted-adhoc-normal-launch",
                "code_provenance": "official-tagged-ipa-instructions-unchanged",
                "signing": "production-adhoc", "push_environment": "production",
                "debugging": "disabled", "original_ipa": "unchanged",
                "entitlement_delta": "beta-reports-active-removal-only",
                "state_policy": "in-place-no-reset-or-restore"}


class ValidationError(ValueError):
    pass


def require(condition):
    if not condition:
        raise ValidationError("Invalid iOS runtime acceptance input")


def exact_object(value, keys):
    require(isinstance(value, dict) and set(value) == set(keys))


def matches(value, pattern):
    return isinstance(value, str) and re.fullmatch(pattern, value) is not None


def digest(value):
    require(matches(value, r"[0-9a-f]{64}"))


def count(value, *, minimum=0, exact=None):
    require(type(value) is int and value >= minimum and (exact is None or value == exact))


def duration(value, minimum):
    require(type(value) in (int, float))
    try:
        require(math.isfinite(value) and value >= minimum)
    except OverflowError:
        raise ValidationError("Invalid iOS runtime acceptance input") from None


def bounded(value, minimum, maximum):
    duration(value, minimum)
    require(value <= maximum)


def validate_adhoc_proof(proof, candidate):
    exact_object(proof, ADHOC_DIGESTS | set(ADHOC_POLICY) |
                 {"ipa_sha256", "bundle_instruction_sha256", "testflight_upgrade_claimed",
                  "installed_binary_extraction_claimed"})
    for field in ADHOC_DIGESTS:
        digest(proof[field])
    for field, expected in ADHOC_POLICY.items():
        require(proof[field] == expected)
    require(proof["ipa_sha256"] == candidate["ipa_sha256"])
    exact_object(proof["bundle_instruction_sha256"], {"app", "notification-service", "share-extension"})
    for value in proof["bundle_instruction_sha256"].values():
        digest(value)
    require(proof["testflight_upgrade_claimed"] is False)
    require(proof["installed_binary_extraction_claimed"] is False)


def validate_quality(quality, candidate):
    exact_object(quality, {"cpu", "startup", "send"})
    cpu = quality["cpu"]
    exact_object(cpu, {"build", "status", "method", "evidence_sha256", "qualification_sha256",
                       "warmup_seconds", "discarded_interval_seconds", "windows",
                       "foreground_sample_max_gap_seconds", "process_sample_max_gap_seconds",
                       "invalid_observations", "process_replacements", "thermal_state"})
    require(cpu["build"] == candidate["apple_build"] and cpu["status"] == "passed")
    require(cpu["method"] == "installed-foreground-cpu-v1" and cpu["thermal_state"] == "nominal")
    digest(cpu["evidence_sha256"])
    digest(cpu["qualification_sha256"])
    # Match the qualified installed-app evaluator, including metric-clock tolerance.
    bounded(cpu["warmup_seconds"], 179.9, 183)
    bounded(cpu["discarded_interval_seconds"], 59.9, 63)
    bounded(cpu["foreground_sample_max_gap_seconds"], 0, 2.5)
    bounded(cpu["process_sample_max_gap_seconds"], 0, 8)
    count(cpu["invalid_observations"], exact=0)
    count(cpu["process_replacements"], exact=0)
    require(isinstance(cpu["windows"], list) and len(cpu["windows"]) == 2)
    for window in cpu["windows"]:
        exact_object(window, {"observed_seconds", "metric_seconds", "cpu_percent_one_core",
                              "disk_write_mib_per_minute"})
        bounded(window["observed_seconds"], 59.9, 63)
        bounded(window["metric_seconds"], 57, 72)
        require(abs(window["observed_seconds"] - window["metric_seconds"]) <= 2)
        bounded(window["cpu_percent_one_core"], 0, 5)
        bounded(window["disk_write_mib_per_minute"], 0, 5)
    startup = quality["startup"]
    exact_object(startup, {"build", "status", "flow", "login_flash_observed",
                           "observations_sha256", "regression_sha256", "review_sha256"})
    require(startup["build"] == candidate["apple_build"])
    require(startup["status"] == "accepted-after-review")
    require(startup["flow"] == "cold-start-to-preserved-state")
    require(startup["login_flash_observed"] is False)
    for field in ("observations_sha256", "regression_sha256", "review_sha256"):
        digest(startup[field])
    send = quality["send"]
    exact_object(send, {"build", "status", "send_to_visible_seconds", "observations_sha256", "review_sha256"})
    require(send["build"] == candidate["apple_build"] and send["status"] == "accepted-after-review")
    # Record observed responsiveness and its review; do not invent a latency budget.
    duration(send["send_to_visible_seconds"], 0)
    digest(send["observations_sha256"])
    digest(send["review_sha256"])


def validate_candidate(candidate):
    exact_object(candidate, RELEASE_KEYS)
    require(matches(candidate["tag"], r"v[0-9]{4}\.[1-9][0-9]?\.[1-9][0-9]?(?:\.[1-9][0-9]?)?"))
    require(matches(candidate["source_commit"], r"[0-9a-f]{40}"))
    digest(candidate["ipa_sha256"])
    digest(candidate["manifest_sha256"])
    require(matches(candidate["apple_version"], r"[0-9]+\.[0-9]+\.[0-9]+"))
    require(matches(candidate["apple_build"], r"[0-9]+") and int(candidate["apple_build"]) > 0)


def validate_receipt(receipt, **candidate):
    """Return only this exact schema; no raw private evidence belongs here."""
    validate_candidate(candidate)
    exact_object(receipt, {"schema_version", "contract", "release", "installation", "checks", "crash_check", "quality"})
    require(type(receipt["schema_version"]) is int and receipt["schema_version"] == 3)
    require(receipt["contract"] == CONTRACT)
    exact_object(receipt["release"], RELEASE_KEYS)
    require(receipt["release"] == candidate)
    installation = receipt["installation"]
    require(isinstance(installation, dict))
    require(installation.get("channel") in ("testflight", ADHOC_CHANNEL))
    extra = {"production_adhoc"} if installation["channel"] == ADHOC_CHANNEL else set()
    exact_object(installation, {"channel", "device", "previous_build", "installed_build", "storage",
                                "account", "history", "draft", "evidence_sha256"} | extra)
    if extra:
        validate_adhoc_proof(installation["production_adhoc"], candidate)
    for field, expected in {"device": "physical-iphone",
                            "installed_build": candidate["apple_build"], "storage": "preserved-app-data-and-keychain",
                            "account": "preserved", "history": "preserved", "draft": "preserved"}.items():
        require(installation[field] == expected)
    previous = installation["previous_build"]
    require(matches(previous, r"[0-9]+"))
    require(0 < int(previous) < int(candidate["apple_build"]))
    digest(installation["evidence_sha256"])
    exact_object(receipt["checks"], PHASES)
    for name, (minimum, state) in PHASES.items():
        phase = receipt["checks"][name]
        exact_object(phase, PHASE_KEYS | STARTUP_PHASE_KEYS.get(name, set()))
        if name == "affected_flow" and phase["process_state"] == "background-suspended":
            minimum, state = 135, "background-suspended"
        if name == "cold_background_launch":
            require(phase["entry_state"] == "background")
            count(phase["foreground_activations"], exact=0)
        elif name == "early_background":
            require(phase["restore_in_flight"] is True)
        require(phase["status"] == "passed" and phase["process_state"] == state)
        duration(phase["observed_seconds"], minimum)
        count(phase["process_samples"], minimum=2)
        count(phase["unexpected_exits"], exact=0)
        count(phase["process_replacements"], exact=0)
        digest(phase["evidence_sha256"])
    crash = receipt["crash_check"]
    exact_object(crash, {"status", "unexpected_reports", "unexpected_exits", "evidence_sha256"})
    require(crash["status"] == "passed")
    count(crash["unexpected_reports"], exact=0)
    count(crash["unexpected_exits"], exact=0)
    digest(crash["evidence_sha256"])
    validate_quality(receipt["quality"], candidate)
    return {"schema_version": 3, "contract": CONTRACT,
            "release": {key: candidate[key] for key in sorted(RELEASE_KEYS)},
            "installation": dict(installation),
            "checks": {name: dict(receipt["checks"][name]) for name in PHASES}, "crash_check": dict(crash),
            "quality": {name: dict(receipt["quality"][name]) for name in ("cpu", "startup", "send")}}


def validate_run(metadata, repository, source_commit):
    """Validate actual GitHub API metadata, without publishing its extra fields."""
    require(matches(repository, r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+"))
    require(matches(source_commit, r"[0-9a-f]{40}"))
    require(isinstance(metadata, dict))
    expected = {"status": "completed", "conclusion": "success", "event": "workflow_dispatch",
                "head_sha": source_commit, "name": WORKFLOW_NAME}
    for field, value in expected.items():
        require(metadata.get(field) == value)
    head_repository = metadata.get("head_repository")
    require(isinstance(head_repository, dict) and head_repository.get("full_name") == repository)
    path = metadata.get("path")
    require(path == WORKFLOW or matches(path, re.escape(WORKFLOW) + r"@refs/(?:heads|tags)/[A-Za-z0-9_./-]+"))
    return {**expected, "head_repository": {"full_name": repository}, "path": WORKFLOW}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def read_json(path):
    with Path(path).open("rb") as handle:
        data = handle.read(MAX_JSON_BYTES + 1)
    require(len(data) <= MAX_JSON_BYTES)
    def invalid_constant(_value):
        raise ValidationError("Invalid iOS runtime acceptance input")
    return json.loads(data.decode("utf-8"), object_pairs_hook=unique_object, parse_constant=invalid_constant)


class Parser(argparse.ArgumentParser):
    def error(self, _message):
        self.exit(2, "Invalid iOS runtime acceptance input.\n")


def main(argv=None):
    parser = Parser(description=__doc__)
    parser.add_argument("--receipt", required=True, type=Path)
    for field in sorted(RELEASE_KEYS):
        parser.add_argument("--" + field.replace("_", "-"), required=True)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--run-metadata", type=Path)
    parser.add_argument("--repository")
    args = parser.parse_args(argv)
    try:
        require(bool(args.run_metadata) == bool(args.repository))
        candidate = {field: getattr(args, field) for field in RELEASE_KEYS}
        normalized = validate_receipt(read_json(args.receipt), **candidate)
        if args.run_metadata:
            validate_run(read_json(args.run_metadata), args.repository, args.source_commit)
        encoded = json.dumps(normalized, sort_keys=True, indent=2, allow_nan=False) + "\n"
        if args.out:
            with args.out.open("x", encoding="utf-8") as handle:
                handle.write(encoded)
        sys.stdout.write(encoded)
        return 0
    except (OSError, ValueError, TypeError, OverflowError, RecursionError):
        print("Invalid iOS runtime acceptance input.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
