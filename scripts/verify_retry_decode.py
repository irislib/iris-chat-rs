#!/usr/bin/env python3
"""Decode one existing capped capture. No tests, recording or permission changes."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import zipfile

import verify_retry_profile as profile

retry = profile.retry
INPUTS = Path(__file__).with_name("retry_decode_inputs.json")
REPO = "irislib/iris-chat-rs"


def contract():
    admission = json.loads(INPUTS.read_text())
    assert admission["enabled"] is True, "Decode-only diagnostic has not been admitted"
    assert admission["schema"] == "iris-retry-decode-input/v1"
    assert admission["profile_input_sha256"] == retry.sha(profile.INPUTS)
    assert admission["profile_helper_sha256"] == retry.sha(Path(profile.__file__))
    data = profile.contract()
    assert admission["source"] == data["source"] and admission["tree"] == data["tree"]
    assert admission["upstream"]["run"] == 39994819299 and admission["upstream"]["artifact"] == 12519178606
    assert admission["upstream"]["driver"] == "0c094caadac1e506e4ade47b3bd383074f0b861a"
    assert admission["binary_sha256"] == "248489ce5e151afcd5d4f99ba4f41a68409c5e168ad88b9bc9a6f8d825c4c6d7"
    assert admission["upstream"]["raw_sha256"] == "2d605075dccc1682608b369d50ae0e6410003c168cd8b7828f0eec7854835dae"
    assert admission["diagnostic_only"] is True and admission["cpu_acceptance"] is False
    data["decode"] = admission
    return data


def limited(command, maximum):
    return [sys.executable, "-c", "import os,resource,sys; n=int(sys.argv[1]); "
            "resource.setrlimit(resource.RLIMIT_FSIZE,(n,n)); os.execv(sys.argv[2],sys.argv[2:])",
            str(maximum), *command]


def upstream(root, output, pre, target, data):
    expected = data["decode"]["upstream"]
    def execute(command, name, timeout=30):
        status, text = retry.bounded(command, root, output, name, timeout, pre["deadline_unix"], target)
        assert status["exit_code"] == 0, f"{name} failed"
        return text
    run = json.loads(execute(["gh", "api", f"repos/{REPO}/actions/runs/{expected['run']}"], "upstream-run"))
    assert (run["id"], run["head_sha"], run["run_attempt"], run["status"], run["conclusion"]) == (expected["run"], expected["driver"], 1, "completed", "failure")
    artifact = json.loads(execute(["gh", "api", f"repos/{REPO}/actions/artifacts/{expected['artifact']}"], "upstream-artifact"))
    assert artifact["id"] == expected["artifact"] and artifact["workflow_run"]["id"] == expected["run"]
    assert artifact["workflow_run"]["head_sha"] == expected["driver"] and artifact["expired"] is False
    assert artifact["name"] == f"retry-profile-{expected['run']}"
    assert artifact["size_in_bytes"] == expected["zip_bytes"] and artifact["digest"] == "sha256:" + expected["zip_sha256"]
    scratch = target.parent / "retry-profile-input"  # Excluded from uploaded evidence.
    scratch.mkdir(exist_ok=False)
    archive = scratch / "upstream.zip"
    writer = "import subprocess,sys; f=open(sys.argv[1],'wb'); subprocess.run(sys.argv[2:],stdout=f,check=True,timeout=45); f.close()"
    execute(limited([sys.executable, "-c", writer, str(archive), "gh", "api", "--allow-escape-sequences",
                     f"repos/{REPO}/actions/artifacts/{expected['artifact']}/zip"], expected["zip_bytes"]), "upstream-download", 60)
    assert archive.stat().st_size == expected["zip_bytes"] and retry.sha(archive) == expected["zip_sha256"]
    with zipfile.ZipFile(archive) as zipped:
        assert len(zipped.namelist()) == len(set(zipped.namelist()))
        raw_info = zipped.getinfo("batched/whole-retry.perf.data")
        assert raw_info.file_size == expected["raw_bytes"]
        raw = scratch / "whole-retry.perf.data"
        with zipped.open(raw_info) as src, raw.open("wb") as dest:
            shutil.copyfileobj(src, dest, 1024 * 1024)
        assert retry.sha(raw) == expected["raw_sha256"]
        for member, name, digest in [("profile-result.json", "upstream-profile-result.json", expected["profile_result_sha256"]),
                                     ("batched/perf-report.log", "upstream-statistics.log", expected["statistics_sha256"])]:
            assert zipped.getinfo(member).file_size < 128 * 1024
            path = output / name; path.write_bytes(zipped.read(member)); assert retry.sha(path) == digest
        preflight = json.loads(zipped.read("preflight.json"))
        assert preflight["driver_source"] == expected["driver"]
    prior = json.loads((output / "upstream-profile-result.json").read_text())
    assert prior["source"]["commit"] == data["source"] and prior["source"]["tree"] == data["tree"]
    assert prior["binary_sha256"] == data["decode"]["binary_sha256"] and prior["cpu_acceptance"] is False
    return raw, prior, {"run": run, "artifact": artifact, "verified": expected}


def decoded(root, out, data, binary, source, build, versions, cargo_profile, pre, output, target):
    receipt = {"schema": "iris-retry-decode-result/v1", "diagnostic_complete": False,
        "diagnostic_only": True, "cpu_acceptance": False, "test_executions": 0,
        "recording_executions": 0, "permission_changes": 0, "source": source,
        "build": build, "toolchain": versions, "cargo_test_profile": cargo_profile}
    try:
        assert retry.sha(binary) == data["decode"]["binary_sha256"], "Rebuilt executable differs from capture"
        receipt["binary_sha256"] = retry.sha(binary)
        raw, prior, proof = upstream(root, output, pre, target, data)
        receipt["upstream"] = proof
        old_command = prior["sampling"]["run"]["command"]
        assert str(binary) == old_command[old_command.index("--") + 1], "Capture executable path changed"
        perf = shutil.which("perf"); assert perf, "perf is unavailable; no tool installation"
        command = [perf, "report", "--stdio", "--header", "--no-children", "--show-nr-samples",
                   "--percent-limit", "0.5", "-g", "graph,0.5,caller", "--no-inline", "-i", str(raw)]
        report, text = retry.bounded(limited(command, 8 * 1024**2), root, output,
                                     "perf-callers", 60, pre["deadline_unix"], target)
        receipt["report"] = report
        assert report["exit_code"] == 0 and (output / "perf-callers.log").stat().st_size < 8 * 1024**2, "Caller report failed or reached output ceiling"
        assert "Overhead" in text and "Samples" in text, "No sample histogram was decoded"
        assert retry.sha(binary) == receipt["binary_sha256"] and retry.sha(raw) == data["decode"]["upstream"]["raw_sha256"]
        assert not retry.git(root, "status", "--porcelain")
        receipt.update(diagnostic_complete=True, interpretation="Existing capped full-test capture only; completion remains failed. Exclusive symbol samples and inclusive caller graph branches use different denominators. Resolve exit_batch ancestry before attributing retry flushes; setup and final persist/reload are separate. 0.5% display threshold and 4 KiB captured stacks limit caller attribution.")
    except BaseException as error:
        receipt["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        retry.save(out / "decode-result.json", receipt)


def run(which, root, out):
    data = contract(); retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data, "batched")
    else:
        assert which == "batched" and root is not None
        retry.stage("batched", root, out, data, source_prover=profile.qualification.source_proof,
                    diagnostic_runner=lambda *args: decoded(root, out, data, *args))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("preflight", "batched"))
    parser.add_argument("--mode", choices=("batched",), default="batched")
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(); out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    try:
        run(args.stage, args.source.resolve() if args.source else None, out)
    except BaseException as error:
        retry.save(out / f"{args.stage}-failure.json", {"cpu_acceptance": False, "error": f"{type(error).__name__}: {error}"})
        raise
