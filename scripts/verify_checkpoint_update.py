#!/usr/bin/env python3
"""Disabled storage RED, focused GREEN, then one realistic v3 CPU qualification."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import subprocess

import verify_inbound_retry as retry

INPUTS = Path(__file__).with_name("checkpoint_update_inputs.json")
STORAGE = "chat-protocol/src/storage/incremental.rs"
SECTION = b"#[cfg(test)]\nmod tests {"
STORAGE_TESTS_SHA256 = "283f5a24f18173e73b8a4fd4f31d9e2b98c3bce925621178495166158631286b"
RED_TEST = "storage::incremental::tests::comparison_reads_use_64k_blocks_without_changing_page_writes"
FAILURE = "checkpoint comparison must read 64 KiB blocks"
INTEGRITY = "protocol_engine::tests::corrupt_mac_inbound_"
FIXTURE_PATHS = ["chat-protocol/src/protocol_engine.rs", "chat-protocol/src/protocol_engine/batched_retry_cpu_tests.rs",
    "chat-protocol/src/protocol_engine/inbound_retry_backlog_tests.rs", "chat-protocol/src/protocol_engine/inbound_retry_ciphertext_fixture_tests.rs"]
GATE_PATHS = ["scripts/test_rust.sh", "scripts/test_rust_runner.py"]
READS = [(0, 65536), (65536, 65536), (131072, 4113)]
WRITES = [(0, 4096), (61440, 4096), (65536, 4096), (135168, 17)]


def spec(name, count=1, exact=True, marker=None, ignored=False, timeout=300):
    return {"name": name, "expected_count": count, "exact": exact, "marker": marker,
            "ignored": ignored, "timeout_seconds": timeout}


def focused_tests():
    return [spec("storage::incremental::tests::", 9, False), spec(INTEGRITY, 2, False),
        spec("protocol_engine::tests::deferred_inbound_deadline_scans_are_bounded_across_32_continuations"),
        spec("protocol_engine::tests::inbound_deadline_", 4, False),
        spec("protocol_engine::tests::remote_retry_", 4, False),
        spec("protocol_engine::tests::remote_direct_", 6, False),
        spec("protocol_engine::tests::group_fanout_retry_allocations_do_not_scale_with_deferred_fanouts", marker="fanout_queue_retry turns=8 due=64 total=24990 small_allocated_bytes="),
        spec("protocol_engine::tests::group_fanout_retry_mixed_updates_restore_order_after_failed_save"),
        spec("protocol_engine::tests::group_fanout_retry_retires_ineligible_deferred_entries_beyond_due_limit"),
        spec("protocol_engine::tests::group_fanout_retry_processes_a_bounded_due_batch"),
        spec("protocol_engine::incoming_retry_tests::pending_receive_save_failure_preserves_entire_queue_and_delivery_journal"),
        spec("protocol_engine::incoming_retry_tests::invalid_pending_ciphertext_does_not_starve_other_senders")]


def cpu_plan():
    return {"test": spec("protocol_engine::tests::restored_backlog_batched_retry_stays_within_cpu_budget", marker="batched_retry_cpu ", ignored=True, timeout=420),
            "failure_marker": "batched steady retry loop still exceeds 5% of one CPU:", "schema_version": 3}


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Checkpoint qualification disabled pending review"
    assert data["schema"] == "iris-checkpoint-update-input/v1"
    chain = data["source_chain"]
    refs = [data["base"], *[item["commit"] for item in chain]]
    assert len(chain) == 6 and len(set(refs)) == 7
    assert all(re.fullmatch(r"[0-9a-f]{40}", ref or "") for ref in refs)
    assert (data["before"], data["after"]) == (chain[0]["commit"], chain[-1]["commit"])
    assert [item["changed_paths"] for item in chain] == [[STORAGE], [STORAGE], FIXTURE_PATHS, ["RELEASE_NOTES.md"], GATE_PATHS, ["RELEASE.md"]]
    assert [item["parent"] for item in chain] == refs[:-1]
    assert data["storage_test_module_sha256"] == STORAGE_TESTS_SHA256
    assert data["before_test"] == RED_TEST and data["before_failure_marker"] == FAILURE
    assert data["before_observation_test"] is None and data["before_observation_failure_marker"] is None
    assert data["tests"] == focused_tests(), "Reviewed 32-test selection changed"
    assert data["cpu"] == cpu_plan(), "Reviewed single v3 CPU test changed"
    return data


def source_bytes(root, commit, path):
    return subprocess.check_output(["git", "-C", str(root), "show", f"{commit}:{path}"], timeout=30)


def module_digest(contents):
    assert contents.count(SECTION) == 1, "Require one exact storage test module boundary"
    return hashlib.sha256(SECTION + contents.split(SECTION)[1]).hexdigest()


def source_proof(which, root, data):
    chain = data["source_chain"][:1] if which == "before" else data["source_chain"]
    proof = []
    for item in chain:
        commit, parent = item["commit"], item["parent"]
        assert retry.git(root, "rev-list", "--parents", "-n", "1", commit).split() == [commit, parent], "Unexpected reviewed source ancestry"
        changed = retry.git(root, "diff", "--name-only", parent, commit).splitlines()
        assert sorted(changed) == sorted(item["changed_paths"]), "Source exceeds reviewed per-commit scope"
        assert module_digest(source_bytes(root, commit, STORAGE)) == data["storage_test_module_sha256"], "Storage regression test module changed"
        proof.append({**item, "tree": retry.git(root, "rev-parse", f"{commit}^{{tree}}"),
            "changed_file_sha256": {path: hashlib.sha256(source_bytes(root, commit, path)).hexdigest() for path in changed}})
    tip = chain[-1]
    assert retry.git(root, "rev-parse", "HEAD") == tip["commit"]
    assert retry.git(root, "rev-parse", "HEAD^") == tip["parent"]
    assert module_digest((root / STORAGE).read_bytes()) == data["storage_test_module_sha256"]
    return {"commit": tip["commit"], "parent": tip["parent"], "changed_paths": tip["changed_paths"],
        "tree": retry.git(root, "rev-parse", "HEAD^{tree}"), "protocol_tree": retry.git(root, "rev-parse", "HEAD:chat-protocol"),
        "fixtures": {STORAGE + "#test-module": data["storage_test_module_sha256"]}, "reviewed_chain": proof,
        "protocol_lock_sha256": retry.sha(root / "chat-protocol/Cargo.lock")}


def check_result(text, test, code, expected_failure=None):
    counts = retry.check_result(text, test, code, expected_failure)
    if test["name"] == RED_TEST:
        assert expected_failure == FAILURE
        vectors = re.findall(r"(?:^|\n)\s*(left|right): (\[[^\n]+\])", text)
        assert [key for key, _ in vectors] == ["left", "right"], "Require exact read vectors from the intended assertion"
        observed, expected = [ast.literal_eval(value) for _, value in vectors]
        assert observed == [(offset, min(4096, 135185 - offset)) for offset in range(0, 135185, 4096)], "RED must exercise original 4 KiB reads"
        assert expected == READS
        counts["checkpoint_reads"] = {"observed": observed, "expected": expected, "logical_bytes": 135185}
    if test["name"] == "storage::incremental::tests::":
        counts["checkpoint_geometry_assertions"] = {"reads": READS, "writes": WRITES,
            "logical_read_bytes": 135185, "changed_write_bytes": 12305, "second_pass_writes": 0}
    if test["name"] == INTEGRITY:
        rows = re.findall(r"corrupt_mac_inbound_fixture packets=(\d+) format_checked=(\d+) decoded_min=(\d+) decoded_max=(\d+) authenticated_failures=(\d+) skip_limit_failures=(\d+) positive_controls=(\d+) distinct_ids=(\d+)", text)
        assert len(rows) == 1, "Require complete actual ciphertext integrity counts"
        packets, checked, minimum, maximum, auth, skipped, controls, ids = map(int, rows[0])
        assert (packets, checked, auth, skipped, controls, ids) == (4214, 4214, 1001, 3213, 2, 4214)
        assert 387 <= minimum <= maximum <= 2115
        retries = re.findall(r"corrupt_mac_inbound_retry packets=(\d+) initially_due=(\d+) rounds=(\d+) saves_on_failed_retry=(\d+) restart_preserved=(true|false)", text)
        assert retries == [("16", "3", "2", "0", "true")], "Retry preservation marker changed"
        counts["inbound_integrity"] = {"packets": packets, "format_checked": checked, "decoded_min": minimum,
            "decoded_max": maximum, "authenticated_failures": auth, "skip_limit_failures": skipped,
            "positive_controls": controls, "distinct_ids": ids, "retry_restart_preserved": True}
    return counts


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data)
    else:
        assert root is not None
        plan = {"tests": [spec(RED_TEST)]} if which == "before" else {"tests": data["tests"], "batched_cpu": data["cpu"]}
        retry.stage(which, root, out, data, release_build=True, result_checker=check_result,
                    source_prover=source_proof, test_plan=plan)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "before", "after"))
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    try:
        run(args.stage, args.source.resolve() if args.source else None, out, contract())
    except BaseException as error:
        retry.save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
