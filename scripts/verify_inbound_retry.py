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


def contract():
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
    return data


def preflight(out, data):
    root = Path(__file__).resolve().parent.parent
    assert not git(root, "status", "--porcelain"), "Driver must be a clean committed checkout"
    receipt = {"inputs": data, "inputs_sha256": sha(INPUTS), "driver_source": git(root, "rev-parse", "HEAD"),
               "toolchain": TOOLCHAIN, "deadline_unix": time.time() + 1500, "workflow_timeout_seconds": 1800}
    save(out / "preflight.json", receipt)
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        stream.write(f"before={data['before']}\nafter={data['after']}\n")


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


def stage(which, root, out, data):
    pre = json.loads((out / "preflight.json").read_text())
    assert pre["inputs"] == data and pre["inputs_sha256"] == sha(INPUTS)
    assert git(root, "rev-parse", "HEAD") == data[which] and not git(root, "status", "--porcelain")
    if which == "after":
        assert json.loads((out / "before/receipt.json").read_text())["accepted"] is True
    parent = data["base"] if which == "before" else data["before"]
    assert git(root, "rev-parse", "HEAD^") == parent, "Unexpected source ancestry"
    changed = git(root, "diff", "--name-only", "HEAD^", "HEAD").splitlines()
    expected_changed = data["before_changed_paths"] if which == "before" else data["after_changed_paths"]
    assert sorted(changed) == sorted(expected_changed), "Source exceeds reviewed diagnostic scope"
    source = {"commit": data[which], "parent": parent, "changed_paths": changed, "tree": git(root, "rev-parse", "HEAD^{tree}"),
              "protocol_tree": git(root, "rev-parse", "HEAD:chat-protocol"),
              "fixtures": {path: sha(root / path) for path in data["fixture_paths"]},
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
    if which == "before":
        save(out / "cache.json", {key: os.environ.get(key, "") for key in ("IRIS_CACHE_REQUESTED", "IRIS_CACHE_MATCHED", "IRIS_CACHE_HIT")})
        # Other branch/tag caches may be inaccessible. Seed the exact locked
        # dependencies once; both source compilations below remain offline.
        fetch_command = ["cargo", f"+{TOOLCHAIN}", "fetch", "--manifest-path", "chat-protocol/Cargo.toml", "--locked"]
        fetch, _ = bounded(fetch_command, root, output, "fetch", 300, pre["deadline_unix"], target)
        assert fetch["exit_code"] == 0, "Dependency fetch failed; no regression result"
    command = ["cargo", f"+{TOOLCHAIN}", "test", "--manifest-path", "chat-protocol/Cargo.toml", "--locked", "--offline", "--lib", "--no-run", "--message-format=json", "--jobs", "2"]
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
    assert profile["opt_level"] == "0" and profile["debug_assertions"] is True and profile["test"] is True
    binary = Path(artifacts[0]["executable"]).resolve()
    assert binary.is_file(), "Test executable is missing"
    assert binary.is_relative_to(target), "Test binary is outside the shared target"
    binary_sha256 = sha(binary)
    save(output / "test-binary.json", {"path": str(binary), "sha256": binary_sha256, "cargo_test_profile": profile,
         "cpu_interpretation": "Default test opt0/debug diagnostic, not release-build equivalence. Deadline mix describes the seeded snapshot; actual eligibility advances during setup and the measured minute."})
    selected = data["tests"] if which == "after" else [t for t in data["tests"] if t["name"] in (data["before_test"], data["before_observation_test"])]
    checks = []
    for index, test in enumerate(selected):
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
            elif which == "before" and run["exit_code"] != 0:
                expected_failure = data["before_observation_failure_marker"]
            if test["name"] == data["before_observation_test"]:
                check["whole_retry_cpu"] = whole_retry_metrics(text)
            check["counts"] = check_result(text, test, run["exit_code"], expected_failure)
            check["accepted"] = True
        except BaseException as error:
            check["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            save(output / "test-results.json", checks)
    assert not git(root, "status", "--porcelain"), "Tests changed the source checkout"
    receipt = {"accepted": True, "stage": which, "source": source, "source_path": str(root), "target_path": str(target),
               "test_binary_sha256": binary_sha256, "build": build, "tests": checks, "toolchain": versions,
               "cargo_test_profile": profile, "physical_cpu_acceptance": False,
               "cpu_interpretation": "Default test profile, opt-level 0 with debug assertions; diagnostic comparison only, not release-build CPU equivalence."}
    save(output / "receipt.json", receipt)
    print(json.dumps({"accepted": True, "stage": which, "source": data[which], "test_counts": [t["counts"] for t in checks]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "before", "after"))
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    try:
        inputs = contract()
        if args.stage == "preflight":
            preflight(out, inputs)
        else:
            assert args.source is not None
            stage(args.stage, args.source.resolve(), out, inputs)
    except BaseException as error:
        save(out / f"{args.stage}-failure.json", {"accepted": False, "error": f"{type(error).__name__}: {error}"})
        raise
