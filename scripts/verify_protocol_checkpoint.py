#!/usr/bin/env python3
"""Green-only protocol suite, durable backlog and unchanged CPU qualification."""
import argparse
import json
import os
from pathlib import Path
import re
import sys

import verify_checkpoint_update as checkpoint
import verify_inbound_retry as retry

INPUTS = Path(__file__).with_name("protocol_checkpoint_inputs.json")
BACKLOG = "protocol_engine::tests::checkpoint_backlog_production_counts_are_durable"
BACKLOG_MARKER = "checkpoint_backlog fanouts=22702 ciphertexts=19231 checkpoint_bytes=254672896"
BINDINGS = {"chat-protocol/Cargo.toml", "chat-protocol/Cargo.lock", "scripts/test_rust.sh",
    "chat-protocol/src/protocol_engine/batched_retry_cpu_tests.rs",
    "chat-protocol/src/protocol_engine/batched_retry_durable_work.rs",
    "chat-protocol/src/protocol_engine/inbound_retry_backlog_tests.rs",
    "chat-protocol/src/protocol_engine/inbound_retry_ciphertext_fixture_tests.rs"}


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Protocol qualification is not admitted"
    assert data["schema"] == "iris-protocol-checkpoint-input/v1"
    assert all(re.fullmatch(r"[0-9a-f]{40}", data[key] or "") for key in ("source", "tree"))
    assert data["toolchain"] == retry.TOOLCHAIN == "1.99.1"
    assert data["resource_helper_sha256"] == retry.sha(Path(retry.__file__))
    assert set(data["bindings"]) == BINDINGS
    assert all(re.fullmatch(r"[0-9a-f]{64}", value) for value in data["bindings"].values())
    assert type(data["ordinary_passed"]) is int and data["ordinary_passed"] >= 188
    assert type(data["ordinary_ignored"]) is int and data["ordinary_ignored"] == 3
    assert data["failed_cpu_evidence"] == {
        "run": 40001081771, "source": "bbb16dc1f6e0209738d86d8c4e9502915ff5c8d3",
        "full_log_sha256": "0494259092e3fd2b83ff9f31c0d7d35eb9d9eae504c37b14941df27e3867239a",
        "reexecute": False}
    ordinary = checkpoint.spec("", data["ordinary_passed"], exact=False)
    ordinary["expected_ignored"] = data["ordinary_ignored"]
    # These compatibility fields are consumed by the unchanged shared stage;
    # no before stage is selected or executed.
    data.update(base=data["failed_cpu_evidence"]["source"], before=data["failed_cpu_evidence"]["source"],
        after=data["source"], before_test=None, before_observation_test=None, tests=[])
    data["batched"] = {"source": data["source"], **checkpoint.cpu_plan(),
        "correctness_tests": [ordinary, checkpoint.spec(BACKLOG, marker=BACKLOG_MARKER, ignored=True)]}
    return data


def source_proof(which, root, data):
    assert which == "batched"
    assert retry.git(root, "rev-parse", "HEAD") == data["source"]
    assert retry.git(root, "rev-parse", "HEAD^{tree}") == data["tree"]
    actual = {path: retry.sha(root / path) for path in data["bindings"]}
    assert actual == data["bindings"], "Reviewed source/dependency/fixture bytes changed"
    return {"commit": data["source"], "tree": data["tree"],
        "protocol_tree": retry.git(root, "rev-parse", "HEAD:chat-protocol"),
        "fixtures": actual, "protocol_lock_sha256": actual["chat-protocol/Cargo.lock"],
        "failed_cpu_evidence": data["failed_cpu_evidence"], "red_executed": False}


def check_result(text, test, code, expected_failure=None):
    if test["name"]:
        return retry.check_result(text, test, code, expected_failure)
    assert code == 0 and expected_failure is None, "Ordinary protocol suite failed"
    rows = re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", text)
    passed, ignored = test["expected_count"], test["expected_ignored"]
    assert rows == [("ok", str(passed), "0", str(ignored), "0", "0")], "Whole protocol suite count changed"
    assert f"running {passed + ignored} tests\n" in text
    return {"passed": passed, "failed": 0, "ignored": ignored, "filtered": 0,
        "expected_regression_failure": False, "whole_nonignored_protocol_suite": True}


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data, "batched")
        return
    retry.stage("batched", root, out, data, result_checker=check_result, source_prover=source_proof)
    # Reapply the exact permanent acceptance parser after the shared stage's
    # strict five-slice checks. A failing stage never reaches this acceptance.
    script = (root / "scripts/test_rust.sh").read_text()
    code = script.split("<<'BATCHED_CPU_RECEIPT'\n", 1)[1].split("\nBATCHED_CPU_RECEIPT", 1)[0]
    pre = json.loads((out / "preflight.json").read_text())
    status, text = retry.bounded([sys.executable, "-c", code, str(out / "batched/test-2.log")],
        root, out, "permanent-cpu-parser", 30, pre["deadline_unix"], Path(os.environ["CARGO_TARGET_DIR"]))
    assert status["exit_code"] == 0 and text.strip() == "Batched protocol CPU, progress and restart evidence verified"


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
        retry.save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
