#!/usr/bin/env python3
"""Disabled authentic bad-MAC session allocation RED/GREEN qualification."""
import argparse
import json
from pathlib import Path
import re

import verify_ndr_blind_allocation as ndr

INPUTS = Path(__file__).with_name("ndr_session_allocation_inputs.json")
TARGET = "session_receive_allocations"
TEST = "far_ahead_receive_avoids_per_key_allocations_and_preserves_retry_state"
FIXTURE = f"rust/crates/nostr-double-ratchet/tests/{TARGET}.rs"
PRODUCTION = [f"rust/crates/nostr-double-ratchet/src/{name}.rs" for name in ("lib", "sender_key", "session", "utils")]


def selected_tests():
    rows = [("test", TARGET, TEST, 1, True),
        ("lib", "nostr_double_ratchet", "utils::tests::fixed_pair_derivation_matches_generic_hkdf", 1, True),
        ("lib", "nostr_double_ratchet", "session::tests::", 6, False),
        ("test", "session_test", "", 7, False),
        ("test", "session_invite_basic", "", 9, False),
        ("test", "session_invite_adversarial", "", 14, False),
        ("test", "session_manager_receive_fallback", "", 2, False),
        ("test", "metadata_privacy", "", 2, False)]
    return [{"kind": kind, "target": target, "name": name, "expected_count": count, "exact": exact,
        "marker": "direct_receive_allocations " if target == TARGET else None, "ignored": False, "timeout_seconds": 300}
        for kind, target, name, count, exact in rows]


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "NDR session allocation diagnostic disabled pending review"
    assert data["repository"] == "irislib/nostr-double-ratchet"
    assert all(re.fullmatch(r"[0-9a-f]{40}", data[key] or "") for key in ("base", "before", "after"))
    assert len({data[key] for key in ("base", "before", "after")}) == 3
    assert data["before_changed_paths"] == [FIXTURE]
    assert data["after_changed_paths"] == PRODUCTION
    assert data["tests"] == selected_tests(), "Reviewed 42-test session correctness selection changed"
    return data


def allocation_result(text, test, code, red):
    rows = re.findall(r"direct_receive_allocations skipped_keys=(\d+) allocations=(\d+) maximum=(\d+) invalid_hmac=(true|false) state_unchanged=(true|false) restored_retry=(true|false) skipped_recovery=(true|false)(?:\r?\n|$)", text)
    assert len(rows) == 1 and text.count("direct_receive_allocations skipped_keys=") == 1, "Missing or duplicate authentic receive allocation metric"
    skipped, allocations, maximum = map(int, rows[0][:3])
    assert skipped == 1000 and maximum == 500 and allocations > 0, "Session allocation workload or fixed guard changed"
    assert rows[0][3:] == ("true", "true", "true", "true"), "Authentication, state, or positive recovery controls failed"
    assert (allocations > maximum) == red, "Allocation result does not match required RED/GREEN stage"
    failure = f"direct receive allocated {allocations} times while deriving skipped keys" if red else None
    counts = ndr.check_result(text, test, code, failure)
    return {"skipped_keys": skipped, "allocations": allocations, "maximum": maximum,
        "invalid_hmac": True, "state_unchanged": True, "restored_retry": True, "skipped_recovery": True}, counts


def session_case():
    return {"schema": "ndr-session-allocation/v1", "inputs_path": INPUTS,
        "allocation_target": TARGET, "allocation_path": FIXTURE, "allocation_result": allocation_result,
        "interpretation": "Authentic MAX_SKIP=1000 bad-MAC session allocation and focused correctness proof only; no CPU measurement or release binary equivalence."}


def run(which, root, out, data):
    if which == "preflight":
        ndr.preflight(out, data, case=session_case())
    else:
        assert root is not None
        ndr.stage(which, root, out, data, case=session_case())


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
        ndr.save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
