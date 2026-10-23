#!/usr/bin/env python3
"""One exact-source sampled retry workload; never a CPU qualification."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

import inbound_retry_perf as perf
import verify_checkpoint_update as checkpoint
import verify_inbound_retry as retry
import verify_protocol_checkpoint as qualification

INPUTS = Path(__file__).with_name("retry_profile_inputs.json")
PARANOID = Path("/proc/sys/kernel/perf_event_paranoid")


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Sampled diagnostic has not been admitted"
    assert data["schema"] == "iris-retry-profile-input/v1"
    assert data["diagnostic_only"] is True and data["cpu_acceptance"] is False
    assert data["source"] == "8a9db73c4d7f259a4f3e28640ed1c1584ff9bc14"
    assert data["tree"] == "200d709495d1f4c926ab13f3179299214512121c"
    assert data["toolchain"] == retry.TOOLCHAIN == "1.99.1"
    assert set(data["bindings"]) == qualification.BINDINGS
    assert all(re.fullmatch(r"[0-9a-f]{64}", value) for value in data["bindings"].values())
    assert set(data["helpers"]) == {"verify_inbound_retry.py", "inbound_retry_perf.py", "verify_checkpoint_update.py", "verify_protocol_checkpoint.py"}
    assert all(retry.sha(INPUTS.with_name(name)) == digest for name, digest in data["helpers"].items())
    assert data["failed_cpu_evidence"] == {
        "run": 39991368237, "source": data["source"], "reexecute_unprofiled": False,
        "result_sha256": "476590acb5d3930f747292b8ea5783ad15941918da4081bb01baa372e732a632",
        "full_log_sha256": "9bd34a7f921b357b7e4c16ba1f668162323b855dc80f150ecf91b3989accb824"}
    assert data["sampling"] == {"frequency_hz": 199, "timeout_seconds": 420,
        "uninline_report": True, "report_max_bytes": 8 * 1024**2}
    data.update(base=data["source"], before=data["source"], after=data["source"],
                before_test=None, before_observation_test=None, tests=[])
    data["batched"] = {"source": data["source"], **checkpoint.cpu_plan(), "correctness_tests": []}
    return data


def permission(out, restore=False):
    path = out / "perf-permission.json"
    if restore and not path.exists():
        return
    assert os.geteuid() != 0, "Only sysctl may use elevated privileges"
    state = json.loads(path.read_text()) if restore else {"original": PARANOID.read_text().strip(), "requested": "2", "restored": False}
    assert re.fullmatch(r"-?\d+", state["original"])
    retry.save(path, state)  # Record restoration input before attempting a change.
    value = state["original"] if restore else "2"
    subprocess.run(["sudo", "-n", "sysctl", "-w", f"kernel.perf_event_paranoid={value}"], check=True, timeout=15)
    observed = PARANOID.read_text().strip()
    state["restored" if restore else "configured"] = observed == value
    state["after_restore" if restore else "after_prepare"] = observed
    retry.save(path, state)
    assert observed == value, "Perf permission setting did not read back exactly"


def sampled(root, out, data, binary, source, build, versions, profile, pre, output, target):
    permission_state = json.loads((out / "perf-permission.json").read_text())
    assert permission_state.get("configured") is True and PARANOID.read_text().strip() == "2"
    assert os.geteuid() != 0, "The workload must run as the ordinary runner user"
    test = data["batched"]["test"]
    command = [str(binary), test["name"], "--nocapture", "--test-threads=1", "--exact", "--ignored"]
    receipt = {"schema": "iris-retry-profile-result/v1", "diagnostic_complete": False,
        "diagnostic_only": True, "cpu_acceptance": False, "physical_cpu_acceptance": False,
        "source": source, "binary_sha256": retry.sha(binary), "build": build,
        "toolchain": versions, "cargo_test_profile": profile,
        "failed_cpu_evidence": data["failed_cpu_evidence"],
        "permission": permission_state,
        "interpretation": "Sampled full test including setup/restart. Top-level perf overhead is exclusive sampled CPU; caller graphs are inclusive and must not be added. Save-caller-filtered samples are not an isolated measured-minute CPU percentage. Phase counters remain wall time. Frequency, lost samples and 4 KiB stack truncation limit attribution."}
    try:
        sampling = perf.sample_failure(command, root, output, pre["deadline_unix"], target,
                                       retry.bounded, retry.sha, **data["sampling"])
        sampling["no_system_settings_changed"] = False
        sampling["permission_change"] = "Ephemeral runner only; always-cleanup restores recorded original setting"
        receipt["sampling"] = sampling
        assert sampling["status"] == "sampled", sampling.get("reason", "Sampling did not execute")
        assert sampling["report"]["exit_code"] == 0, "Perf report failed or exceeded its 8 MiB output ceiling; raw capture retained"
        assert sampling["report"]["stop_reason"] is None and sampling["run"]["stop_reason"] is None
        text = (output / sampling["test_log"]).read_text()
        metrics = retry.batched_retry_metrics(text, schema_version=3)
        exceeds = any(row["cpu_percent_one_core"] > 5 for row in metrics["slices"][-2:])
        counts = retry.check_result(text, test, sampling["run"]["exit_code"], data["batched"]["failure_marker"] if exceeds else None)
        counts["expected_regression_failure"] = False
        receipt.update(diagnostic_complete=True, test_counts=counts, sampled_cpu_exceeds_budget=exceeds,
                       batched_retry_cpu=metrics)
        assert retry.sha(binary) == receipt["binary_sha256"] and not retry.git(root, "status", "--porcelain")
    except BaseException as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        retry.save(out / "profile-result.json", receipt)


def run(which, root, out):
    if which == "permission-restore":
        permission(out, restore=True)
        return
    data = contract()
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data, "batched")
    elif which == "permission-prepare":
        permission(out)
    else:
        assert which == "batched" and root is not None
        retry.stage("batched", root, out, data, source_prover=qualification.source_proof,
                    diagnostic_runner=lambda *args: sampled(root, out, data, *args))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "batched", "permission-prepare", "permission-restore"))
    parser.add_argument("--mode", choices=("batched",), default="batched")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    try:
        run(args.stage, args.source.resolve() if args.source else None, out)
    except BaseException as error:
        retry.save(out / f"{args.stage}-failure.json", {"diagnostic_only": True, "cpu_acceptance": False, "error": f"{type(error).__name__}: {error}"})
        raise
