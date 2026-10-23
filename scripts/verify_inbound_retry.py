#!/usr/bin/env python3
"""Diagnostic branch only: exact hosted before/after protocol regression."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

TOOLCHAIN = "1.99.1"
GIB = 1024**3
INPUTS = Path(__file__).with_name("inbound_retry_inputs.json")


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], timeout=30).decode().strip()


def contract(mode="comparison"):
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Diagnostic disabled: exact source/fixture owner inputs are not admitted"
    assert all(re.fullmatch(r"[0-9a-f]{40}", data[key] or "") for key in ("base", "before", "after"))
    assert len({data[key] for key in ("base", "before", "after")}) == 3
    assert data["fixture_paths"] and all(p.startswith("chat-protocol/") and ".." not in Path(p).parts for p in data["fixture_paths"])
    tests = data["tests"]
    assert len(tests) >= 3 and len({t["name"] for t in tests}) == len(tests)
    assert all(re.fullmatch(r"[A-Za-z0-9_:]+", t["name"]) and 0 < t["timeout_seconds"] <= 300 and isinstance(t["ignored"], bool) and isinstance(t["exact"], bool) and t["expected_count"] > 0 for t in tests)
    assert data["before_test"] in {t["name"] for t in tests} and data["before_failure_marker"]
    assert data["before_observation_test"] in {t["name"] for t in tests} and data["before_observation_failure_marker"]
    if mode == "optimized":
        assert data["optimized_enabled"] is True, "Optimized diagnostic disabled pending review"
    if mode == "batched":
        candidate = data["batched"]
        assert candidate["enabled"] is True, "Batched diagnostic disabled pending review"
        assert all(re.fullmatch(r"[0-9a-f]{40}", candidate[key] or "") for key in ("source", "base"))
        assert candidate["source"] != candidate["base"]
        reviewed_gate_paths = {"RELEASE.md", "scripts/test_rust.sh", "scripts/test_rust_runner.py"}
        assert candidate["changed_paths"] and all((p.startswith("chat-protocol/") or p in reviewed_gate_paths) and ".." not in Path(p).parts for p in candidate["changed_paths"])
        test = candidate["test"]
        assert re.fullmatch(r"[A-Za-z0-9_:]+", test["name"]) and test["exact"] is True and test["ignored"] is True
        assert test["expected_count"] == 1 and test["timeout_seconds"] == 420 and test["marker"] == "batched_retry_cpu "
        correctness = candidate["correctness_tests"]
        assert correctness and len({t["name"] for t in [*correctness, test]}) == len(correctness) + 1
        assert all(re.fullmatch(r"[A-Za-z0-9_:]+", t["name"]) and t["timeout_seconds"] == 300 and t["ignored"] is False and isinstance(t["exact"], bool) and t["expected_count"] > 0 for t in correctness)
        assert candidate["failure_marker"]
    return data


def preflight(out, data, mode="comparison"):
    root = Path(__file__).resolve().parent.parent
    assert not git(root, "status", "--porcelain"), "Driver must be a clean committed checkout"
    receipt = {"inputs": data, "inputs_sha256": sha(INPUTS), "driver_source": git(root, "rev-parse", "HEAD"),
               "mode": mode, "toolchain": TOOLCHAIN, "deadline_unix": time.time() + 1500, "workflow_timeout_seconds": 1800}
    save(out / "preflight.json", receipt)
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        stream.write(f"before={data['before']}\nafter={data['after']}\n")
        if mode == "batched":
            stream.write(f"batched={data['batched']['source']}\n")


def check_result(text, test, code, expected_failure=None):
    results = re.findall(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)
    expected = ("FAILED", "0", "1", "0") if expected_failure else ("ok", str(test["expected_count"]), "0", "0")
    assert results == [expected], f"Exact test count/result mismatch: {results}"
    assert f"running {test['expected_count']} test" in text and test["name"] in text, "Selected test did not execute"
    if test["marker"]:
        assert test["marker"] in text, "Required fixture execution marker absent"
    assert code == (101 if expected_failure else 0), f"Unexpected test process exit {code}"
    if expected_failure:
        assert expected_failure in text and "panicked at" in text, "Failure was not the intended regression assertion"
    return {"passed": int(expected[1]), "failed": int(expected[2]), "ignored": 0,
            "expected_regression_failure": bool(expected_failure), "execution_marker": test["marker"]}


def whole_retry_metrics(text):
    rows = re.findall(r"whole_retry_cpu (\{[^\n]+\})", text)
    assert len(rows) == 1, "Expected exactly one whole retry measurement"
    result = json.loads(rows[0])
    expected = {"fixture": "restored-mixed-backlog-v1", "synthetic": True,
                "pending_inbound": 4214, "initially_due_inbound": 102,
                "fanouts": 24990, "initially_due_fanouts": 10462,
                "ciphertexts": 23437, "checkpoint_bytes": 350224384, "remote_sends": 44,
                "continuation_delay_ms": 25, "liveness_delay_ms": 2000, "outer_batch": False,
                "all_ciphertext_authors_have_group_state": True, "group_deliveries": 1,
                "remaining_inbound": 4214, "remaining_ciphertexts": 23436}
    assert all(result[key] == value for key, value in expected.items()), "Whole retry workload/readiness changed"
    assert result["elapsed_seconds"] >= 60 and result["continuations"] >= 8 and result["puts"] > 0
    assert result["remaining_fanouts"] >= 24990
    assert math.isfinite(result["cpu_percent_one_core"]) and result["cpu_percent_one_core"] >= 0
    return result


def batched_retry_metrics(text):
    rows = re.findall(r"batched_retry_cpu (\{[^\n]+\})", text)
    assert len(rows) == 1, "Expected exactly one complete batched retry receipt"
    result = json.loads(rows[0])
    expected = {"schema": "iris-batched-retry-cpu/v2", "fixture": "restored-mixed-backlog-with-expiring-controls-v2", "synthetic": True,
                "pending_inbound": 4214, "initially_due_inbound": 102, "fanouts": 24990,
                "initially_due_fanouts": 10462, "ciphertexts": 23437, "remote_sends": 44,
                "expiring_controls": 2, "initial_remote_sends": 46, "retired_expiring_controls": 2,
                "checkpoint_bytes": 350224384, "outer_batch": True, "ack_and_flush_in_turn": True,
                "continuation_delay_ms": 25, "liveness_delay_ms": 2000, "settle_seconds": 180,
                "measurement_windows": 2, "window_seconds": 60, "debug_assertions": False,
                "initially_ready": True, "group_deliveries": 1, "acknowledged_deliveries": 2,
                "acknowledged_setup_deliveries": 1, "acknowledged_group_deliveries": 1,
                "repeated_search_starts": 0, "wire_effect_count": 0, "remaining_inbound": 4214,
                "remaining_ciphertexts": 23436, "remaining_remote_sends": 44,
                "preservation_checked": True, "restart_checked": True, "restart_ready": True}
    assert all(result[key] == value for key, value in expected.items()), "Batched workload or completion assertions changed"
    assert result["remaining_fanouts"] >= 24990 and result["search_starts"] > 0
    assert all(isinstance(result[key], (int, float)) and math.isfinite(result[key]) and 0 <= result[key] <= 180
               for key in ("restore_seconds", "readiness_seconds", "first_valid_delivery_seconds")), "Recovery deadline not met"
    slices = result["slices"]
    emitted = [json.loads(row) for row in re.findall(r"batched_retry_slice (\{[^\n]+\})", text)]
    assert len(slices) == 5 and emitted == slices, "Require five complete independently emitted slices"
    progress_counts = ("search_starts", "repeated_search_starts", "input_generation_changes", "ready_candidates", "engine_attempts")
    phase_keys = {"retirement", "group_inputs", "group_fanouts", "inbound", "remote_sends", "local_siblings", "flush", "decrypt_cursor_nested"}
    previous = None
    elapsed_sum = 0.0
    for index, row in enumerate(slices):
        assert (row["stage"], row["stage_index"]) == (("settle", index) if index < 3 else ("measure", index - 3)), "Slice order changed"
        for key in ("elapsed_seconds", "recovery_elapsed_seconds", "cpu_seconds", "cpu_percent_one_core", "work_seconds", "max_turn_seconds"):
            assert isinstance(row[key], (int, float)) and math.isfinite(row[key]) and row[key] >= 0, f"Invalid slice metric {key}"
        assert row["elapsed_seconds"] >= 60
        elapsed_sum += row["elapsed_seconds"]
        assert row["recovery_elapsed_seconds"] + 0.001 >= elapsed_sum
        assert math.isclose(row["cpu_percent_one_core"], 100 * row["cpu_seconds"] / row["elapsed_seconds"], rel_tol=1e-8, abs_tol=1e-8)
        phases = row["phase_work_seconds"]
        assert phases.keys() == phase_keys, "Retry phase timer fields changed"
        assert all(type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in phases.values()), "Invalid retry phase time"
        # These are wall times, not CPU samples. The decrypt cursor is nested
        # within group_inputs and must not be added to total work a second time.
        assert sum(value for key, value in phases.items() if key != "decrypt_cursor_nested") <= row["work_seconds"] + 0.01, "Retry phase times exceed total work"
        assert phases["decrypt_cursor_nested"] <= phases["group_inputs"] + 0.01, "Nested decrypt time exceeds group-input time"
        for key in ("turns", "continuations", "acknowledged_deliveries", "puts", "read_bytes", "written_bytes", "remote_deadline_updates", "expired_control_retirements"):
            assert type(row[key]) is int and row[key] >= 0, f"Invalid slice counter {key}"
        assert row["expired_control_retirements"] == (0 if index < 3 else 1), "Expiring controls did not retire once per measured slice"
        assert all(type(row["due_at_start"][key]) is int and row["due_at_start"][key] >= 0 for key in ("inbound", "fanouts", "remote_sends"))
        before, after = row["progress_before"], row["progress_after"]
        for progress in (before, after):
            assert all(type(progress[key]) is int and progress[key] >= 0 for key in progress_counts)
            assert all(type(progress[key]) is bool for key in ("active_cursor", "prepared_result"))
            assert progress["repeated_search_starts"] == 0, "Candidate re-admitted in unchanged input generation"
        assert all(after[key] >= before[key] for key in ("search_starts", "input_generation_changes"))
        assert previous is None or before == previous, "Audit/progress reset between slices"
        previous = after
        if index >= 3:
            assert row["turns"] > 0 and row["remote_deadline_updates"] > 0 and row["puts"] > 0, "Measured slice did not exercise recurring due work and flush"
    assert sum(row["acknowledged_deliveries"] for row in slices) == 2
    assert sum(row["expired_control_retirements"] for row in slices) == result["retired_expiring_controls"]
    assert all(result[key] == previous[key] for key in ("search_starts", "repeated_search_starts", "input_generation_changes"))
    assert type(result["restart_ready"]) is bool
    restart = result["restart_progress"]
    assert all(type(restart[key]) is int and restart[key] >= 0 for key in progress_counts)
    assert all(type(restart[key]) is bool for key in ("active_cursor", "prepared_result"))
    assert restart["search_starts"] == restart["repeated_search_starts"] == 0, "Restart readiness performed a key search"
    return result


def bounded(command, root, out, name, timeout, deadline, target):
    """Use the source-package driver's owned process-group/resource pattern."""
    log = out / f"{name}.log"
    memory = int(next(row.split()[1] for row in Path("/proc/meminfo").read_text().splitlines() if row.startswith("MemTotal:"))) * 1024
    limit = min(5 * GIB, memory * 4 // 5)
    assert limit >= 4 * GIB, "Runner has inadequate memory for the synthetic backlog"
    minimum_free = shutil.disk_usage(target).free
    assert minimum_free >= 5 * GIB, "Less than 5 GiB free before stage"
    started, peak, reason, process, caught = time.monotonic(), 0, None, None, None
    try:
        assert time.time() < deadline, "Comparison deadline reached"
        with log.open("wb") as stream, (out / f"{name}-resources.jsonl").open("w") as samples:
            process = subprocess.Popen(command, cwd=root, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            next_report = 0
            while process.poll() is None:
                elapsed = time.monotonic() - started
                rows = subprocess.check_output(["ps", "-eo", "pgid=,rss="], timeout=10).decode().splitlines()
                rss = sum(int(parts[1]) * 1024 for row in rows if len(parts := row.split()) == 2 and int(parts[0]) == process.pid)
                peak = max(peak, rss)
                free = shutil.disk_usage(target).free
                minimum_free = min(minimum_free, free)
                if elapsed >= timeout or time.time() >= deadline:
                    reason = "Stage or comparison time limit reached"
                elif rss > limit:
                    reason = "Owned process group exceeded memory budget"
                elif free < 2 * GIB:
                    reason = "Free disk fell below 2 GiB"
                if elapsed >= next_report or reason:
                    row = {"stage": name, "seconds": round(elapsed, 1), "rss_bytes": rss, "free_bytes": free, "stop_reason": reason}
                    samples.write(json.dumps(row) + "\n"); samples.flush()
                    print(json.dumps(row), flush=True)
                    next_report = elapsed + 30
                if reason:
                    break
                time.sleep(1)
    except BaseException as error:
        caught = error
        reason = f"{type(error).__name__}: {error}"
    finally:
        if process is not None and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=10)
        result = {"command": command, "exit_code": process.returncode if process else None,
                  "elapsed_seconds": round(time.monotonic() - started, 2), "timeout_seconds": timeout,
                  "peak_rss_bytes": peak, "memory_limit_bytes": limit, "minimum_free_disk_bytes": minimum_free,
                  "stop_reason": reason, "owned_process_reaped": process is not None and process.returncode is not None,
                  "log_sha256": sha(log) if log.exists() else None}
        save(out / f"{name}-status.json", result)
    assert reason is None, f"Bounded stage failed: {reason}"
    if caught:
        raise caught
    return result, log.read_text(errors="replace")


def stage(which, root, out, data, *, release_build=False, result_checker=None):
    result_checker = result_checker or check_result
    batched = which == "batched"
    optimized = which in ("optimized", "batched")
    source_key = "after" if optimized else which
    source_commit = data["batched"]["source"] if batched else data[source_key]
    pre = json.loads((out / "preflight.json").read_text())
    assert pre["inputs"] == data and pre["inputs_sha256"] == sha(INPUTS)
    assert pre["mode"] == (which if optimized else "comparison")
    assert git(root, "rev-parse", "HEAD") == source_commit and not git(root, "status", "--porcelain")
    if which == "after":
        assert json.loads((out / "before/receipt.json").read_text())["accepted"] is True
    parent = data["base"] if which == "before" else data["before"]
    if batched:
        parent = data["batched"]["base"]
    assert git(root, "rev-parse", "HEAD^") == parent, "Unexpected source ancestry"
    changed = git(root, "diff", "--name-only", "HEAD^", "HEAD").splitlines()
    expected_changed = data["before_changed_paths"] if which == "before" else data["after_changed_paths"]
    if batched:
        expected_changed = data["batched"]["changed_paths"]
    assert sorted(changed) == sorted(expected_changed), "Source exceeds reviewed diagnostic scope"
    source = {"commit": source_commit, "parent": parent, "changed_paths": changed, "tree": git(root, "rev-parse", "HEAD^{tree}"),
              "protocol_tree": git(root, "rev-parse", "HEAD:chat-protocol"),
              "fixtures": {path: sha(root / path) for path in (expected_changed if batched else data["fixture_paths"])},
              "protocol_lock_sha256": sha(root / "chat-protocol/Cargo.lock")}
    output = out / which
    output.mkdir()
    save(output / "source.json", source)
    versions = {}
    for tool in ("rustc", "cargo"):
        text = subprocess.check_output([tool, f"+{TOOLCHAIN}", "--version", "--verbose"], timeout=60).decode()
        assert re.search(rf"^{tool} {re.escape(TOOLCHAIN)}(?:\s|$)", text)
        versions[tool] = text
    save(output / "toolchain.json", versions)
    if which == "after":
        before = json.loads((out / "before/source.json").read_text())
        assert source["fixtures"] == before["fixtures"], "Regression fixture bytes changed between before and after"
        assert source["protocol_lock_sha256"] == before["protocol_lock_sha256"], "Dependency inputs changed"
        assert versions == json.loads((out / "before/toolchain.json").read_text())
    target = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    target.mkdir(parents=True, exist_ok=True)
    if which == "before" or optimized:
        save(out / "cache.json", {key: os.environ.get(key, "") for key in ("IRIS_CACHE_REQUESTED", "IRIS_CACHE_MATCHED", "IRIS_CACHE_HIT")})
        # Other branch/tag caches may be inaccessible. Seed the exact locked
        # dependencies once; both source compilations below remain offline.
        fetch_command = ["cargo", f"+{TOOLCHAIN}", "fetch", "--manifest-path", "chat-protocol/Cargo.toml", "--locked"]
        fetch, _ = bounded(fetch_command, root, output, "fetch", 300, pre["deadline_unix"], target)
        assert fetch["exit_code"] == 0, "Dependency fetch failed; no regression result"
    command = ["cargo", f"+{TOOLCHAIN}", "test", "--manifest-path", "chat-protocol/Cargo.toml", "--locked", "--offline", "--lib", "--no-run", "--message-format=json", "--jobs", "2"]
    if optimized or release_build:
        command.append("--release")
    build, text = bounded(command, root, output, "build", 600, pre["deadline_unix"], target)
    assert build["exit_code"] == 0, "Compilation failed; this is not the expected regression"
    artifacts = []
    for line in text.splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if value.get("reason") == "compiler-artifact" and value.get("executable") and value.get("target", {}).get("name") == "iris_chat_protocol" and value.get("profile", {}).get("test"):
            artifacts.append(value)
    assert len(artifacts) == 1, "Expected exactly one protocol test executable"
    profile = artifacts[0]["profile"]
    assert profile["test"] is True
    if optimized or release_build:
        assert profile["opt_level"] == "3" and profile["debug_assertions"] is False and profile["debuginfo"] == 1
        interpretation = "Release optimization with debug symbols, not official release binary equivalence. "
        if release_build and not optimized:
            interpretation += "Focused correctness comparison only; no CPU measurement."
        else:
            interpretation += "Batched retry/ACK/flush with three settling and two measured slices." if batched else "Standalone public API without the Mac lifecycle outer batch."
    else:
        assert profile["opt_level"] == "0" and profile["debug_assertions"] is True
        interpretation = "Default test opt0/debug diagnostic, not release-build equivalence."
    if not release_build or optimized:
        interpretation += " Deadline mix describes the seeded snapshot; actual eligibility advances during setup and "
        interpretation += "the timed slices." if batched else "the measured minute."
    binary = Path(artifacts[0]["executable"]).resolve()
    assert binary.is_file(), "Test executable is missing"
    assert binary.is_relative_to(target), "Test binary is outside the shared target"
    binary_sha256 = sha(binary)
    save(output / "test-binary.json", {"path": str(binary), "sha256": binary_sha256, "cargo_test_profile": profile,
         "cpu_interpretation": interpretation})
    selected = data["tests"] if which == "after" else [t for t in data["tests"] if t["name"] in (data["before_test"], data["before_observation_test"])]
    if optimized:
        selected = [*data["batched"]["correctness_tests"], data["batched"]["test"]] if batched else [t for t in data["tests"] if t["name"] == data["before_observation_test"]]
        assert selected[-1]["exact"] and selected[-1]["expected_count"] == 1
        assert batched or len(selected) == 1
    checks = []
    cpu_failure = False
    for index, test in enumerate(selected):
        is_cpu_test = optimized and (not batched or test["name"] == data["batched"]["test"]["name"])
        command = [str(binary), test["name"], "--nocapture", "--test-threads=1"]
        if test["exact"]:
            command.append("--exact")
        if test["ignored"]:
            command.append("--ignored")
        run, text = bounded(command, root, output, f"test-{index}", test["timeout_seconds"], pre["deadline_unix"], target)
        check = {"test": test, "run": run, "accepted": False}
        checks.append(check)
        try:
            expected_failure = None
            if which == "before" and test["name"] == data["before_test"]:
                expected_failure = data["before_failure_marker"]
            elif (which == "before" or is_cpu_test) and run["exit_code"] != 0:
                expected_failure = data["batched"]["failure_marker"] if batched else data["before_observation_failure_marker"]
            if batched and is_cpu_test:
                check["batched_retry_cpu"] = batched_retry_metrics(text)
            elif test["name"] == data["before_observation_test"]:
                check["whole_retry_cpu"] = whole_retry_metrics(text)
            check["counts"] = result_checker(text, test, run["exit_code"], expected_failure)
            cpu_failure = is_cpu_test and expected_failure is not None
            if is_cpu_test:
                exceeds_budget = any(s["cpu_percent_one_core"] > 5 for s in check["batched_retry_cpu"]["slices"][-2:]) if batched else check["whole_retry_cpu"]["cpu_percent_one_core"] > 5
                assert exceeds_budget == cpu_failure, "CPU assertion and metric disagree"
                check["counts"]["expected_regression_failure"] = False
            check["accepted"] = not cpu_failure
        except BaseException as error:
            check["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            save(output / "test-results.json", checks)
    assert not git(root, "status", "--porcelain"), "Tests changed the source checkout"
    assert sha(binary) == binary_sha256, "Test executable changed during measurement"
    receipt = {"accepted": not cpu_failure, "stage": which, "source": source, "source_path": str(root), "target_path": str(target),
               "test_binary_sha256": binary_sha256, "build": build, "tests": checks, "toolchain": versions,
               "cargo_test_profile": profile, "physical_cpu_acceptance": False,
               "cpu_interpretation": interpretation, "unprofiled_cpu_failure": cpu_failure}
    save(output / "receipt.json", receipt)
    print(json.dumps({"accepted": not cpu_failure, "stage": which, "source": source_commit, "test_counts": [t["counts"] for t in checks]}), flush=True)
    if batched and cpu_failure:
        raise AssertionError("Batched settled retry CPU exceeds the unchanged 5% limit; no repeat or sampling run")
    if cpu_failure:
        from inbound_retry_perf import sample_failure
        sampling = sample_failure(command, root, output, pre["deadline_unix"], target, bounded, sha)
        if sampling["status"] == "sampled":
            sampled_text = (output / sampling["test_log"]).read_text(errors="replace")
            try:
                sampled_code = sampling["run"]["exit_code"]
                sampling["test_counts"] = check_result(sampled_text, selected[0], sampled_code,
                    data["before_observation_failure_marker"] if sampled_code else None)
                sampling["whole_retry_cpu"] = whole_retry_metrics(sampled_text)
            except Exception as error:
                sampling["test_result_error"] = f"{type(error).__name__}: {error}"
        save(output / "sampling.json", sampling)
        assert sha(binary) == binary_sha256, "Test executable changed during sampling"
        raise AssertionError("Optimized unprofiled whole retry CPU exceeds the unchanged 5% limit; sampling is diagnostic only")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "before", "after", "optimized", "batched"))
    parser.add_argument("--mode", choices=("comparison", "optimized", "batched"), default="comparison")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    try:
        mode = args.stage if args.stage in ("optimized", "batched") else args.mode
        inputs = contract(mode)
        if args.stage == "preflight":
            preflight(out, inputs, mode)
        else:
            assert args.source is not None
            stage(args.stage, args.source.resolve(), out, inputs)
    except BaseException as error:
        save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
