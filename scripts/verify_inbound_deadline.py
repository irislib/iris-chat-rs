#!/usr/bin/env python3
"""Disabled narrow deadline-scan comparison; no CPU observation or repeat."""
import argparse
import json
from pathlib import Path
import re

import verify_inbound_retry as retry

INPUTS = Path(__file__).with_name("inbound_deadline_inputs.json")
SCAN_TEST = "protocol_engine::tests::deferred_inbound_deadline_scans_are_bounded_across_32_continuations"
FAILURE = "32 continuations inspected "


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Inbound deadline comparison disabled pending review"
    assert all(re.fullmatch(r"[0-9a-f]{40}", data[key] or "") for key in ("base", "before", "after"))
    assert len({data[key] for key in ("base", "before", "after")}) == 3
    for field in ("fixture_paths", "before_changed_paths", "after_changed_paths"):
        assert data[field] and len(data[field]) == len(set(data[field]))
        assert all(p.startswith("chat-protocol/") and ".." not in Path(p).parts for p in data[field])
    assert data["before_test"] == SCAN_TEST and data["before_failure_marker"] == FAILURE
    assert data["before_observation_test"] is None and data["before_observation_failure_marker"] is None
    tests = data["tests"]
    assert [(test["name"], test["expected_count"], test["exact"]) for test in tests] == [
        (SCAN_TEST, 1, True), ("protocol_engine::tests::inbound_deadline_", 4, False),
        ("protocol_engine::tests::remote_retry_", 4, False),
        ("protocol_engine::tests::remote_direct_", 6, False),
        ("protocol_engine::tests::group_fanout_retry_allocations_do_not_scale_with_deferred_fanouts", 1, True),
        ("protocol_engine::tests::group_fanout_retry_mixed_updates_restore_order_after_failed_save", 1, True),
        ("protocol_engine::tests::group_fanout_retry_retires_ineligible_deferred_entries_beyond_due_limit", 1, True),
        ("protocol_engine::tests::group_fanout_retry_processes_a_bounded_due_batch", 1, True),
        ("protocol_engine::incoming_retry_tests::pending_receive_save_failure_preserves_entire_queue_and_delivery_journal", 1, True),
        ("protocol_engine::incoming_retry_tests::invalid_pending_ciphertext_does_not_starve_other_senders", 1, True),
    ], "Reviewed 21-test selection changed"
    assert all(t["timeout_seconds"] == 300 and t["ignored"] is False for t in tests)
    assert all(t["marker"] is None for t in tests if t is not tests[4])
    assert tests[4]["marker"] == "fanout_queue_retry turns=8 due=64 total=24990 small_allocated_bytes="
    return data


def check_result(text, test, code, expected_failure=None):
    counts = retry.check_result(text, test, code, expected_failure)
    if test["name"] == SCAN_TEST:
        if expected_failure:
            assert expected_failure == FAILURE
            rows = re.findall(r"32 continuations inspected (\d+) inbound retry deadlines", text)
            assert len(rows) == 1, "Require the numeric deadline-scan assertion"
            scans = int(rows[0])
            assert scans > 3 * 4214, "RED must exceed the upper scan limit, not fail the fixture's lower bound"
            counts["deadline_scans"] = scans
        else:
            # GREEN does not print a scan metric. A pass proves the exact source
            # assertion's inclusive bounds; do not invent an observed count.
            counts["deadline_scans"] = None
        counts["deadline_scan_guard"] = {"packets": 4214, "initially_due": 102,
            "continuations": 32, "minimum": 4214, "maximum": 3 * 4214}
    return counts


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data)
    else:
        assert root is not None
        retry.stage(which, root, out, data, release_build=True, result_checker=check_result)


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
