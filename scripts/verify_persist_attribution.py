#!/usr/bin/env python3
"""One diagnostic persistence-attribution workload; never release CPU acceptance."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re

import verify_checkpoint_update as checkpoint
import verify_inbound_retry as retry

INPUTS = Path(__file__).with_name("persist_attribution_inputs.json")
BASE = "c49702b174253c6c722e56f2793fcf598f32760f"
PATHS = ["chat-protocol/src/protocol_engine/batched_retry_cpu_tests.rs",
    "chat-protocol/src/protocol_engine/engine_persistence.rs", "chat-protocol/src/protocol_engine/retry_work_audit.rs"]
LOCK_SHA256 = "81357f39ed1994242ff8a46cf52cd4f2fd22d58667f74f4445a0ff4f873ee04c"
TRACE_SCHEMA = "iris-retry-persistence-trace/v1"


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Persistence attribution disabled pending source and driver review"
    assert data["schema"] == "iris-persist-attribution-input/v1"
    assert data["diagnostic_only"] is True and data["release_cpu_acceptance"] is False
    assert data["base"] == data["before"] == BASE
    assert re.fullmatch(r"[0-9a-f]{40}", data["after"] or "") and data["after"] != BASE
    candidate = data["batched"]
    assert (candidate["source"], candidate["base"], candidate["changed_paths"]) == (data["after"], BASE, PATHS)
    assert candidate["correctness_tests"] == data["tests"] == []
    plan = {key: candidate[key] for key in ("test", "failure_marker", "schema_version")}
    assert plan == checkpoint.cpu_plan(), "CPU workload or limits changed"
    assert data["before_test"] is None and data["before_observation_test"] is None
    assert data["trace"] == {"schema": TRACE_SCHEMA, "limit": 64, "overflow_allowed": False}
    assert data["protocol_lock_sha256"] == LOCK_SHA256
    return data


def source_proof(which, root, data):
    assert which == "batched"
    source = data["after"]
    assert retry.git(root, "rev-list", "--parents", "-n", "1", source).split() == [source, BASE]
    changed = retry.git(root, "diff", "--name-only", BASE, source).splitlines()
    assert changed == PATHS, "Instrumentation exceeded reviewed three-file scope"
    assert retry.git(root, "rev-parse", "HEAD") == source
    assert retry.sha(root / "chat-protocol/Cargo.lock") == LOCK_SHA256
    assert hashlib.sha256(checkpoint.source_bytes(root, BASE, "chat-protocol/Cargo.lock")).hexdigest() == LOCK_SHA256
    return {"commit": source, "parent": BASE, "changed_paths": changed,
        "tree": retry.git(root, "rev-parse", "HEAD^{tree}"), "protocol_tree": retry.git(root, "rev-parse", "HEAD:chat-protocol"),
        "fixtures": {path: retry.sha(root / path) for path in changed}, "protocol_lock_sha256": LOCK_SHA256,
        "diagnostic_only": True, "release_cpu_acceptance": False}


def basename(value):
    assert isinstance(value, str) and value and value not in (".", "..")
    assert "/" not in value and "\\" not in value and Path(value).name == value


def traces(metrics):
    total, previous_time, slices = 0, -1.0, []
    for row in metrics["slices"]:
        trace = row["persistence_trace"]
        assert set(trace) == {"schema", "limit", "overflow", "successful_puts", "total_successful_puts", "writes"}
        assert trace["schema"] == TRACE_SCHEMA and type(trace["limit"]) is int and trace["limit"] == 64
        assert trace["overflow"] is False, "Attribution trace overflowed"
        assert type(trace["successful_puts"]) is int and trace["successful_puts"] >= 0
        assert trace["successful_puts"] == len(trace["writes"]) == row["puts"], "Trace writes differ from actual slice puts"
        total += len(trace["writes"])
        assert type(trace["total_successful_puts"]) is int and trace["total_successful_puts"] == total <= 64
        for write in trace["writes"]:
            assert set(write) == {"elapsed_seconds", "batch_depth", "caller_file", "caller_line", "requests"}
            elapsed = write["elapsed_seconds"]
            assert type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= 0 and elapsed >= previous_time
            previous_time = elapsed
            assert type(write["batch_depth"]) is int and 0 <= write["batch_depth"] <= 2**32 - 1
            assert type(write["caller_line"]) is int and 0 < write["caller_line"] <= 2**32 - 1
            basename(write["caller_file"])
            seen = set()
            for request in write["requests"]:
                assert set(request) == {"file", "line", "count"}
                basename(request["file"])
                assert type(request["line"]) is int and 0 < request["line"] <= 2**32 - 1
                assert type(request["count"]) is int and request["count"] > 0
                location = (request["file"], request["line"])
                assert location not in seen, "Repeated requests must be aggregated per write"
                seen.add(location)
        slices.append(trace)
    return {"schema": TRACE_SCHEMA, "limit": 64, "overflow": False, "successful_puts": total, "slices": slices}


def check_result(text, test, code, expected_failure=None):
    counts = retry.check_result(text, test, code, expected_failure)
    metrics = retry.batched_retry_metrics(text, schema_version=3)
    counts["persistence_attribution"] = traces(metrics)
    return counts


def finish_attribution(out):
    path = out / "batched/receipt.json"
    if not path.exists():
        return
    receipt = json.loads(path.read_text())
    tests = receipt["tests"]
    assert len(tests) == 1
    test = tests[0]
    assert test["counts"]["persistence_attribution"] == traces(test["batched_retry_cpu"])
    retry.save(out / "attribution.json", {"schema": "iris-persist-attribution-result/v1",
        "diagnostic_complete": True, "diagnostic_only": True, "release_cpu_acceptance": False,
        "source": receipt["source"], "binary_sha256": receipt["test_binary_sha256"],
        "instrumented_cpu_budget_passed": receipt["accepted"],
        "persistence_attribution": test["counts"]["persistence_attribution"],
        "interpretation": "Instrumented workload for checkpoint-call attribution only. Neither passing CPU timings nor this receipt qualify a release."})


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data, "batched")
    else:
        assert which == "batched" and root is not None
        try:
            retry.stage("batched", root, out, data, result_checker=check_result, source_prover=source_proof)
        finally:
            finish_attribution(out)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "batched"))
    parser.add_argument("--mode", choices=("batched",), default="batched")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    try:
        run(args.stage, args.source.resolve() if args.source else None, out, contract())
    except BaseException as error:
        retry.save(out / f"{args.stage}-failure.json", {"accepted": False, "diagnostic_only": True, "error": f"{type(error).__name__}: {error}"})
        raise
