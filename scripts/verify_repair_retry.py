#!/usr/bin/env python3
"""Disabled exact repair RED, 52 focused GREEN checks, then unchanged v3 CPU."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import tomllib
import re

import verify_checkpoint_update as checkpoint
import verify_inbound_retry as retry
import verify_persist_attribution as attribution
import verify_receive_cache as receive

INPUTS = Path(__file__).with_name("repair_retry_inputs.json")
BASE = "f78dab07f23c89aa9128dc9d2baa68ee5bbace77"
COMMITS = ["8008aee1a29f6b22c0752aa28440ac51f2395f18", "ba57b5fec3b063158f34fa282a5ec2cf8554336c",
    "e4b45261c6d0fbc4971d791d1c61ce0a72b724d6", "ed15fa9c81ff8d34c749b9ed165ecdf05f1c4c16"]
PREFIX = "protocol_engine::tests::repair_retry_"
RED_TEST = PREFIX + "backoff_only_keeps_exact_intent_without_checkpoint"
FAILURE = "repair_retry_persistence_tests.rs:122:5"
DIRECTORY = "chat-protocol/src/protocol_engine/"
TEST_PATH = DIRECTORY + "repair_retry_persistence_tests.rs"
INCLUDE_PATH = DIRECTORY + "repair_response_throttle_tests.rs"
GREEN_PATHS = [DIRECTORY + name for name in ("engine_resolution.rs", "engine_sender_key_repair.rs", "group_retry.rs")]
FIXTURES = {
    TEST_PATH: "1a8a3f6d9ba26c39d7b65f21f035a800857c16b4495675e1087b85fb7a1f53a1",
    INCLUDE_PATH: "88c34a6d80d2bbc58d7d986e90e8884107817897701d76fd0ddebb4bf1a2ac8f",
    DIRECTORY + "batched_retry_cpu_tests.rs": "962081f9f89635a26e4d45e8d92c16973506093bbe0196a5d017db762823dda7",
    DIRECTORY + "batched_retry_durable_work.rs": "25d879908886ae0c53294e6d3aabca0d65a011ef537ef79828fd5eaef1c75dd9",
    DIRECTORY + "inbound_retry_backlog_tests.rs": "6d6796dfe3e49130667e4518e45010a4cee62e163761ea3f8f4a05c101d56f36",
    DIRECTORY + "inbound_retry_ciphertext_fixture_tests.rs": "4e865795edddef4371b8eed0f089b40a4ef87217ed120dc2c70bdcbdb2c88f6a",
    DIRECTORY + "retry_work_audit.rs": "9d8d2e4cd38e3009fed1366a618afe78b9ad1cf540a0f183fa2969095519d5da",
}
DEPENDENCIES = {which: {
    "manifest_sha256": "37b410f2c4b9afa9ef07cac2682ac7261f8c44d8a047351649ed4052a9fb857a",
    "lock_sha256": "81357f39ed1994242ff8a46cf52cd4f2fd22d58667f74f4445a0ff4f873ee04c",
    "ndr_version": "0.0.173", "ndr_checksum": "d838e57c5db5431ccc1e412ea795fed4013e288b7d139c39f29c1c9a2806a3e7",
} for which in ("before", "after")}


def focused_tests():
    names = ["refreshed_sender_key_repair_requests_share_response_backoff",
        "refreshed_repair_requests_do_not_duplicate_an_undeliverable_intent"]
    companions = ["group_receive_checkpoints_only_ready_mutations_not_replays_or_search_slices",
        "group_retry_continuations_do_not_checkpoint_and_failed_final_save_can_replan",
        "group_fanout_retries_only_missing_devices_after_restart",
        "group_fanout_drops_revoked_targets_and_never_adds_new_devices_to_pending_history",
        "group_fanout_save_failure_keeps_unsent_targets_and_ratchets_retryable"]
    return (receive.focused_tests() + [checkpoint.spec(PREFIX, 5, False)]
        + [checkpoint.spec("protocol_engine::tests::" + name) for name in names]
        + [checkpoint.spec("protocol_engine::tests::repair_coalescing_", 3, False)]
        + [checkpoint.spec("protocol_engine::tests::" + name) for name in companions])


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Repair retry qualification disabled pending final source review"
    assert data["schema"] == "iris-repair-retry-input/v1"
    assert (data["base"], data["before"], data["after"]) == (BASE, COMMITS[2], COMMITS[-1])
    chain = data["source_chain"]
    assert [item["commit"] for item in chain] == COMMITS
    assert [item["parent"] for item in chain] == [BASE, *COMMITS[:-1]]
    assert [item["changed_paths"] for item in chain] == [
        [INCLUDE_PATH, TEST_PATH], [TEST_PATH], [TEST_PATH], GREEN_PATHS]
    assert data["fixtures"] == FIXTURES and data["dependencies"] == DEPENDENCIES
    assert data["before_test"] == RED_TEST and data["before_failure_marker"] == FAILURE
    assert data["before_observation_test"] is None and data["before_observation_failure_marker"] is None
    assert data["tests"] == focused_tests(), "Reviewed 52-test selection changed"
    assert data["cpu"] == checkpoint.cpu_plan(), "Unchanged v3 CPU plan required"
    assert data["trace"] == {"schema": attribution.TRACE_SCHEMA, "limit": 64, "overflow_allowed": False}
    return data


def dependencies(root, which, data):
    manifest_bytes = (root / "chat-protocol/Cargo.toml").read_bytes()
    lock_bytes = (root / "chat-protocol/Cargo.lock").read_bytes()
    expected = data["dependencies"][which]
    hashes = {"manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "lock_sha256": hashlib.sha256(lock_bytes).hexdigest()}
    assert hashes == {key: expected[key] for key in hashes}, "Unreviewed manifest or locked dependencies"
    manifest, lock = tomllib.loads(manifest_bytes.decode()), tomllib.loads(lock_bytes.decode())
    assert manifest["dependencies"][receive.NDR] == expected["ndr_version"]
    assert manifest["dependencies"][receive.CODEC] == "0.0.168"
    packages = [p for p in lock["package"] if p["name"].startswith(receive.NDR)]
    assert len(packages) == 2 and {p["name"] for p in packages} == {receive.NDR, receive.CODEC}
    for package in packages:
        assert package["source"] == receive.REGISTRY
        assert package["version"] == (expected["ndr_version"] if package["name"] == receive.NDR else "0.0.168")
        assert package["checksum"] == (expected["ndr_checksum"] if package["name"] == receive.NDR else receive.CODEC_CHECKSUM)
    return {**expected, "packages": {p["name"]: {key: p[key] for key in ("version", "source", "checksum")} for p in packages}}


def source_proof(which, root, data):
    chain = data["source_chain"][:3] if which == "before" else data["source_chain"]
    proof = []
    for index, item in enumerate(chain):
        commit, parent = item["commit"], item["parent"]
        assert retry.git(root, "rev-list", "--parents", "-n", "1", commit).split() == [commit, parent]
        changed = retry.git(root, "diff", "--name-only", parent, commit).splitlines()
        assert changed == item["changed_paths"], "Source exceeds reviewed per-commit scope"
        for path, digest in FIXTURES.items():
            # Initial RED corrections are retained; only final RED runs.
            if index < 2 and path == TEST_PATH:
                continue
            assert hashlib.sha256(checkpoint.source_bytes(root, commit, path)).hexdigest() == digest
        proof.append({**item, "tree": retry.git(root, "rev-parse", commit + "^{tree}"),
            "changed_file_sha256": {path: hashlib.sha256(checkpoint.source_bytes(root, commit, path)).hexdigest() for path in changed}})
    tip = chain[-1]
    assert retry.git(root, "rev-parse", "HEAD") == tip["commit"]
    assert retry.git(root, "rev-parse", "HEAD^") == tip["parent"]
    for path, digest in FIXTURES.items():
        assert retry.sha(root / path) == digest
    assert (root / TEST_PATH).read_text().splitlines()[121].strip() == "assert_eq!(observed, vec![(0, true), (1, false)]);"
    dep = dependencies(root, which, data)
    return {"commit": tip["commit"], "parent": tip["parent"], "changed_paths": tip["changed_paths"],
        "tree": retry.git(root, "rev-parse", "HEAD^{tree}"), "protocol_tree": retry.git(root, "rev-parse", "HEAD:chat-protocol"),
        "fixtures": FIXTURES, "reviewed_chain": proof, "dependencies": dep, "protocol_lock_sha256": dep["lock_sha256"]}


def dependency_transition(before, after, data):
    for which, source in (("before", before), ("after", after)):
        assert source["commit"] == data[which]
        dep, expected = source["dependencies"], data["dependencies"][which]
        assert {key: dep[key] for key in expected} == expected
        assert source["protocol_lock_sha256"] == dep["lock_sha256"]
        assert dep["packages"] == {
            receive.NDR: {"version": expected["ndr_version"], "source": receive.REGISTRY, "checksum": expected["ndr_checksum"]},
            receive.CODEC: {"version": "0.0.168", "source": receive.REGISTRY, "checksum": receive.CODEC_CHECKSUM}}
    return {"accepted": True, "before": before["dependencies"], "after": after["dependencies"],
        "fetch": "One bounded locked fetch per reviewed source lock, followed by offline release builds."}


def check_result(text, test, code, expected_failure=None):
    if test["name"] == RED_TEST:
        counts = retry.check_result(text, test, code, expected_failure)
        assert expected_failure == FAILURE and text.count("panicked at") == 1
        assert "assertion `left == right` failed" in text
        rows = re.findall(r"(?:^|\n)\s*(left|right): (\[[^\n]+\])", text)
        assert [key for key, _ in rows] == ["left", "right"]
        observed, expected = [ast.literal_eval(value.replace("false", "False").replace("true", "True")) for _, value in rows]
        assert observed == [(1, False), (1, False)] and expected == [(0, True), (1, False)], "Require final checkpoint assertion after all positive controls"
        counts["repair_checkpoints"] = {"observed": observed, "expected": expected}
        return counts
    counts = checkpoint.check_result(text, test, code, expected_failure)
    if test == checkpoint.cpu_plan()["test"]:
        counts["persistence_attribution"] = attribution.traces(retry.batched_retry_metrics(text, schema_version=3))
    return counts


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data)
    else:
        assert root is not None
        plan = {"tests": [checkpoint.spec(RED_TEST)]} if which == "before" else {"tests": data["tests"], "batched_cpu": data["cpu"]}
        retry.stage(which, root, out, data, release_build=True, result_checker=check_result,
            source_prover=source_proof, test_plan=plan, dependency_checker=dependency_transition)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "before", "after"))
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    try:
        run(args.stage, args.source.resolve() if args.source else None, out, contract())
    except BaseException as error:
        retry.save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
