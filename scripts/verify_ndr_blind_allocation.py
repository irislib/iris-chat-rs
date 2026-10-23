#!/usr/bin/env python3
"""Temporary diagnostic only: bounded, exact NDR allocation RED/GREEN proof."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

from verify_inbound_retry import TOOLCHAIN, bounded, check_result, git, save, sha

INPUTS = Path(__file__).with_name("ndr_blind_allocation_inputs.json")
MANIFEST = "rust/Cargo.toml"
PACKAGE = "nostr-double-ratchet"
ALLOCATION_TARGET = "sender_key_blind_allocations"
ALLOCATION_TEST = "blind_search_exhaustion_has_constant_bookkeeping_allocations"
ALLOCATION_PATH = f"rust/crates/{PACKAGE}/tests/{ALLOCATION_TARGET}.rs"
PRODUCTION_PATHS = {f"rust/crates/{PACKAGE}/src/sender_key.rs", f"rust/crates/{PACKAGE}/src/sender_key/blind.rs"}


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "NDR allocation diagnostic disabled pending exact source review"
    assert data["repository"] == "irislib/nostr-double-ratchet"
    assert all(re.fullmatch(r"[0-9a-f]{40}", data[key] or "") for key in ("base", "before", "after"))
    assert len({data[key] for key in ("base", "before", "after")}) == 3
    assert set(data["before_changed_paths"]) == PRODUCTION_PATHS | {ALLOCATION_PATH}
    assert set(data["after_changed_paths"]) == PRODUCTION_PATHS
    tests = data["tests"]
    assert len(tests) == 4 and sum(t["expected_count"] for t in tests) == 27
    assert [(t["kind"], t["target"], t["name"], t["expected_count"], t["exact"]) for t in tests] == [
        ("test", ALLOCATION_TARGET, ALLOCATION_TEST, 1, True),
        ("lib", "nostr_double_ratchet", "sender_key::", 17, False),
        ("test", "sender_key_test", "", 3, False),
        ("test", "group_manager_sender_key_basic", "bounded_receive_", 6, False),
    ], "Reviewed correctness selection changed"
    assert all(t["timeout_seconds"] == 300 and t["ignored"] is False for t in tests)
    assert tests[0]["marker"] == "blind_search_allocations "
    assert all(t["marker"] is None for t in tests[1:])
    return data


def preflight(out, data):
    root = Path(__file__).resolve().parent.parent
    assert not git(root, "status", "--porcelain"), "Driver must be a clean committed checkout"
    save(out / "preflight.json", {"schema": "ndr-blind-allocation/v1", "inputs": data,
        "inputs_sha256": sha(INPUTS), "driver_source": git(root, "rev-parse", "HEAD"),
        "toolchain": TOOLCHAIN, "deadline_unix": time.time() + 1500,
        "workflow_timeout_seconds": 1800})
    with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
        stream.write(f"repository={data['repository']}\nbefore={data['before']}\nafter={data['after']}\n")


def allocation_result(text, test, code, red):
    # The metric is printed only after the source-owned exhaustion, exact trial
    # budget, PendingDistribution and pure-state assertions have all passed.
    rows = re.findall(r"blind_search_allocations attempts=(\d+) allocations=(\d+) maximum=(\d+)(?:\r?\n|$)", text)
    assert len(rows) == 1 and text.count("blind_search_allocations attempts=") == 1, "Missing or duplicate allocation metric"
    attempts, allocations, maximum = map(int, rows[0])
    assert attempts == 10001 and maximum == 16, "Allocation fixture workload or threshold changed"
    assert (allocations > maximum) == red, "Allocation result does not match required RED/GREEN stage"
    failure = f"blind-search bookkeeping allocated {allocations} times" if red else None
    counts = check_result(text, test, code, failure)
    return {"attempts": attempts, "allocations": allocations, "maximum": maximum}, counts


def build_command(tests):
    command = ["cargo", f"+{TOOLCHAIN}", "test", "--manifest-path", MANIFEST,
        "-p", PACKAGE, "--locked", "--offline", "--release", "--no-run",
        "--message-format=json", "--jobs", "2"]
    for test in tests:
        command += ["--lib"] if test["kind"] == "lib" else ["--test", test["target"]]
    return command


def test_binaries(text, tests, target):
    required = {(t["kind"], t["target"]) for t in tests}
    binaries = {}
    for line in text.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("reason") != "compiler-artifact" or not row.get("executable") or not row.get("profile", {}).get("test"):
            continue
        kinds = row["target"]["kind"]
        assert len(kinds) == 1
        key = (kinds[0], row["target"]["name"])
        assert key in required and key not in binaries, "Unexpected or duplicate test executable"
        profile = row["profile"]
        assert profile["test"] is True and profile["opt_level"] == "3"
        assert profile["debug_assertions"] is False and profile["debuginfo"] == 1
        binary = Path(row["executable"]).resolve()
        assert binary.is_file() and binary.is_relative_to(target), "Test binary is outside the shared target or missing"
        binaries[key] = {"kind": key[0], "target": key[1], "path": str(binary),
            "sha256": sha(binary), "cargo_test_profile": profile}
    assert binaries.keys() == required, "Missing selected NDR test executable"
    return binaries


def source_proof(which, root, data):
    source, parent = data[which], data["base"] if which == "before" else data["before"]
    assert git(root, "rev-parse", "HEAD") == source and not git(root, "status", "--porcelain")
    assert git(root, "rev-parse", "HEAD^") == parent, "Unexpected source ancestry"
    changed = git(root, "diff", "--name-only", "HEAD^", "HEAD").splitlines()
    assert sorted(changed) == sorted(data[f"{which}_changed_paths"]), "Source exceeds reviewed diagnostic scope"
    dependencies = git(root, "ls-files", "rust/Cargo.lock", "rust/**/Cargo.toml", "rust/Cargo.toml").splitlines()
    assert "rust/Cargo.lock" in dependencies and MANIFEST in dependencies
    return {"commit": source, "parent": parent, "tree": git(root, "rev-parse", "HEAD^{tree}"),
        "changed_paths": changed, "changed_file_sha256": {p: sha(root / p) for p in changed},
        "allocation_fixture_sha256": sha(root / ALLOCATION_PATH),
        "dependency_inputs": {p: sha(root / p) for p in dependencies}}


def require_reuse(before, source, root, target, versions):
    assert before["accepted"] is True and before["stage"] == "before", "GREEN requires accepted assertion-only RED"
    assert before["source_path"] == str(root) and before["target_path"] == str(target), "RED/GREEN paths changed"
    assert before["source"]["commit"] == source["parent"], "GREEN is not the child of measured RED"
    assert before["source"]["allocation_fixture_sha256"] == source["allocation_fixture_sha256"], "Allocation fixture changed"
    assert before["source"]["dependency_inputs"] == source["dependency_inputs"], "Dependency inputs changed"
    assert before["toolchain"] == versions, "Toolchain changed between RED and GREEN"


def stage(which, root, out, data):
    pre = json.loads((out / "preflight.json").read_text())
    assert pre["inputs"] == data and pre["inputs_sha256"] == sha(INPUTS)
    assert pre["schema"] == "ndr-blind-allocation/v1" and pre["toolchain"] == TOOLCHAIN
    assert time.time() < pre["deadline_unix"], "Diagnostic deadline reached"
    output = out / which
    output.mkdir()
    source = source_proof(which, root, data)
    save(output / "source.json", source)
    versions = {}
    for tool in ("rustc", "cargo"):
        version = subprocess.check_output([tool, f"+{TOOLCHAIN}", "--version", "--verbose"], timeout=60).decode()
        assert re.search(rf"^{tool} {re.escape(TOOLCHAIN)}(?:\s|$)", version)
        versions[tool] = version
    save(output / "toolchain.json", versions)
    target = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    target.mkdir(parents=True, exist_ok=True)
    if which == "after":
        require_reuse(json.loads((out / "before/receipt.json").read_text()), source, root, target, versions)
    else:
        save(out / "cache.json", {key: os.environ.get(key, "") for key in ("NDR_CACHE_REQUESTED", "NDR_CACHE_MATCHED", "NDR_CACHE_HIT")})
        fetch, _ = bounded(["cargo", f"+{TOOLCHAIN}", "fetch", "--manifest-path", MANIFEST, "--locked"],
            root, output, "fetch", 300, pre["deadline_unix"], target)
        assert fetch["exit_code"] == 0, "Dependency fetch failed; no regression result"
    selected = data["tests"] if which == "after" else data["tests"][:1]
    build, text = bounded(build_command(selected), root, output, "build", 600, pre["deadline_unix"], target)
    assert build["exit_code"] == 0, "Compilation failed; this is not the expected regression"
    binaries = test_binaries(text, selected, target)
    save(output / "test-binaries.json", list(binaries.values()))
    checks = []
    for index, test in enumerate(selected):
        binary = binaries[(test["kind"], test["target"])]
        command = [binary["path"], "--nocapture", "--test-threads=1"]
        if test["name"]:
            command.append(test["name"])
        if test["exact"]:
            command.append("--exact")
        run, text = bounded(command, root, output, f"test-{index}", test["timeout_seconds"], pre["deadline_unix"], target)
        check = {"test": test, "run": run, "test_binary_sha256": binary["sha256"], "accepted": False}
        checks.append(check)
        try:
            if test["target"] == ALLOCATION_TARGET:
                check["allocation"], check["counts"] = allocation_result(text, test, run["exit_code"], which == "before")
            else:
                check["counts"] = check_result(text, test, run["exit_code"])
            check["accepted"] = True
        except BaseException as error:
            check["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            save(output / "test-results.json", checks)
    assert not git(root, "status", "--porcelain"), "Tests changed the source checkout"
    assert all(sha(Path(b["path"])) == b["sha256"] for b in binaries.values()), "Test executable changed during verification"
    save(output / "receipt.json", {"schema": "ndr-blind-allocation/v1", "accepted": True,
        "stage": which, "source": source, "source_path": str(root), "target_path": str(target),
        "build": build, "tests": checks, "test_binaries": list(binaries.values()), "toolchain": versions,
        "physical_cpu_acceptance": False,
        "interpretation": "Synthetic malformed-payload allocation and focused correctness proof only; no CPU measurement or release binary equivalence."})
    print(json.dumps({"accepted": True, "stage": which, "source": source["commit"],
        "test_counts": [t["counts"] for t in checks]}), flush=True)


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
