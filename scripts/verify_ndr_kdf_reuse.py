#!/usr/bin/env python3
"""Disabled fixed-salt extraction proof and bounded same-host microbenchmark."""
import argparse
import json
import math
import os
from pathlib import Path
import re
import statistics

import verify_ndr_blind_allocation as ndr

INPUTS = Path(__file__).with_name("ndr_kdf_reuse_inputs.json")
PREFIX = "sender_key::blind::kdf_reuse_tests::"
TEST = PREFIX + "blind_kdf_initializes_fixed_salt_once_per_forward_advance"
BENCHMARK = PREFIX + "blind_kdf_corrupt_mac_microbenchmark"
FIXTURE = "rust/crates/nostr-double-ratchet/src/sender_key/kdf_reuse_tests.rs"
PRODUCTION = sorted(ndr.PRODUCTION_PATHS)
FAILURE = "fixed-salt extraction must initialize once per forward advance"


def selected_tests():
    rows = [("lib", "nostr_double_ratchet", TEST, 1, True),
        ("lib", "nostr_double_ratchet", "sender_key::tests::", 11, False),
        ("lib", "nostr_double_ratchet", "sender_key::blind::tests::", 6, False),
        ("lib", "nostr_double_ratchet", PREFIX + "blind_kdf_reference_keys_match_partitioned_boundary_states", 1, True),
        ("lib", "nostr_double_ratchet", PREFIX + "blind_kdf_corrupt_mac_preserves_exhaustion_and_skipped_only_work", 1, True),
        ("test", "sender_key_test", "", 3, False),
        ("test", "group_manager_sender_key_basic", "bounded_receive_", 6, False),
        ("test", "sender_key_blind_allocations", "blind_search_exhaustion_has_constant_bookkeeping_allocations", 1, True)]
    return [{"kind": kind, "target": target, "name": name, "expected_count": count, "exact": exact,
        "marker": "blind_kdf_initializations " if name == TEST else None, "ignored": False, "timeout_seconds": 300}
        for kind, target, name, count, exact in rows]


def benchmark_plan():
    return {"name": BENCHMARK, "expected_count": 1, "exact": True, "ignored": True,
        "marker": "blind_kdf_benchmark ", "timeout_seconds": 300,
        "samples": 7, "searches_per_sample": 16, "trials_per_sample": 160016,
        "budget": 256, "ciphertext_bytes": 1091, "warmup_samples": 1}


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "NDR KDF reuse diagnostic disabled pending review"
    assert data["repository"] == "irislib/nostr-double-ratchet"
    assert (data["base"], data["before"], data["after"]) == (
        "4c52dd532b59f01cf79ac20432b0f5c687d237cb", "b23e4e74c56d9fe0153d112e22540723965aac62", "8e27286c501dcfd838a69f0de15e9d2151dfe8ce")
    assert data["before_changed_paths"] == sorted([*PRODUCTION, FIXTURE])
    assert data["after_changed_paths"] == PRODUCTION
    assert data["tests"] == selected_tests() and data["benchmark"] == benchmark_plan()
    assert data["profiling"] is False and data["release_cpu_acceptance"] is False
    return data


def operation_result(text, test, code, red):
    if test["name"] != TEST:
        assert not red
        return None, ndr.check_result(text, test, code)
    rows = re.findall(r"blind_kdf_initializations trials=(\d+) initializations=(\d+) maximum=(\d+) invalid_hmac=(true|false) state_unchanged=(true|false) reference_state=(true|false)(?:\r?\n|$)", text)
    assert len(rows) == 1 and text.count("blind_kdf_initializations trials=") == 1
    trials, count, maximum = map(int, rows[0][:3])
    assert (trials, count, maximum) == (256, 256 if red else 1, 1)
    assert rows[0][3:] == ("true", "true", "true")
    counts = ndr.check_result(text, test, code, FAILURE if red else None)
    return {"trials": trials, "initializations": count, "maximum": maximum,
        "invalid_hmac": True, "state_unchanged": True, "reference_state": True}, counts


def case():
    return {"schema": "ndr-kdf-reuse/v1", "inputs_path": INPUTS,
        "allocation_target": "nostr_double_ratchet", "allocation_path": FIXTURE,
        "allocation_result": operation_result,
        "interpretation": "Fixed-salt extraction operation count and focused correctness. Separate bounded same-host benchmark is diagnostic only; no application CPU or release acceptance."}


def benchmark_result(text, test, code):
    counts = ndr.check_result(text, test, code)
    rows = re.findall(r"blind_kdf_benchmark sample=(\d+) searches=(\d+) trials=(\d+) budget=(\d+) ciphertext_bytes=(\d+) elapsed_seconds=([^\s]+) invalid_hmac=(true|false) state_unchanged=(true|false)(?:\r?\n|$)", text)
    assert len(rows) == text.count("blind_kdf_benchmark sample=") == 7, "Incomplete or duplicate benchmark samples"
    samples = []
    for index, row in enumerate(rows):
        assert tuple(map(int, row[:5])) == (index, 16, 160016, 256, 1091), "Benchmark work changed"
        seconds = float(row[5]); assert math.isfinite(seconds) and seconds > 0
        assert row[6:] == ("true", "true")
        samples.append(seconds)
    return {"samples_seconds": samples, "median_seconds": statistics.median(samples),
        "minimum_seconds": min(samples), "maximum_seconds": max(samples),
        "median_nanoseconds_per_trial": statistics.median(samples) * 1e9 / 160016}, counts


def benchmark(which, root, out, data):
    pre = json.loads((out / "preflight.json").read_text())
    output = out / which
    receipt = json.loads((output / "receipt.json").read_text())
    assert receipt["accepted"] is True and receipt["source"]["commit"] == data[which]
    binaries = [b for b in receipt["test_binaries"] if (b["kind"], b["target"]) == ("lib", "nostr_double_ratchet")]
    assert len(binaries) == 1
    binary = binaries[0]; path = Path(binary["path"])
    target = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    assert path.is_relative_to(target) and ndr.sha(path) == binary["sha256"]
    test = data["benchmark"]
    command = [str(path), test["name"], "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    run, text = ndr.bounded(command, root, output, "benchmark", test["timeout_seconds"], pre["deadline_unix"], target)
    metrics, counts = benchmark_result(text, test, run["exit_code"])
    assert ndr.sha(path) == binary["sha256"] and not ndr.git(root, "status", "--porcelain")
    result = {"schema": "ndr-kdf-reuse-benchmark/v1", "source": data[which], "test": test,
        "binary_sha256": binary["sha256"], "run": run, "counts": counts, "metrics": metrics,
        "diagnostic_only": True, "release_cpu_acceptance": False}
    ndr.save(output / "benchmark.json", result)
    if which == "after":
        before = json.loads((out / "before/benchmark.json").read_text())
        assert before["source"] == data["before"] and before["test"] == test
        ratio = metrics["median_seconds"] / before["metrics"]["median_seconds"]
        ndr.save(out / "comparison.json", {"schema": "ndr-kdf-reuse-comparison/v1", "before": before, "after": result,
            "median_after_over_before": ratio, "median_reduction_fraction": 1 - ratio,
            "diagnostic_only": True, "release_cpu_acceptance": False,
            "limitations": ["Baseline ran first; samples are sequential on one hosted runner, not randomized or a statistical confidence interval.",
                "The disabled test-only TLS initialization observer is visited per trial in RED versus once per advance in GREEN; these optimized test timings are not production or application CPU proof.",
                "One warmup plus seven fixed samples per source; no profiling, adaptive repeats or additional workloads."]})


def run(which, root, out, data):
    if which == "preflight":
        ndr.preflight(out, data, case=case())
    else:
        assert root is not None
        ndr.stage(which, root, out, data, case=case())
        benchmark(which, root, out, data)


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
        ndr.save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
