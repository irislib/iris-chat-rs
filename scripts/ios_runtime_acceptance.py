#!/usr/bin/env python3
"""Validate a safe, candidate-bound receipt for a distributed iOS upgrade."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys


CONTRACT = "ios-distributed-upgrade-v2"
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
    exact_object(receipt, {"schema_version", "contract", "release", "installation", "checks", "crash_check"})
    require(type(receipt["schema_version"]) is int and receipt["schema_version"] == 2)
    require(receipt["contract"] == CONTRACT)
    exact_object(receipt["release"], RELEASE_KEYS)
    require(receipt["release"] == candidate)
    installation = receipt["installation"]
    exact_object(installation, {"channel", "device", "previous_build", "installed_build", "storage",
                                "account", "history", "draft", "evidence_sha256"})
    for field, expected in {"channel": "testflight", "device": "physical-iphone",
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
    return {"schema_version": 2, "contract": CONTRACT,
            "release": {key: candidate[key] for key in sorted(RELEASE_KEYS)},
            "installation": dict(installation),
            "checks": {name: dict(receipt["checks"][name]) for name in PHASES}, "crash_check": dict(crash)}


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
