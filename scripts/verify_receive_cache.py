#!/usr/bin/env python3
"""Disabled failed-receive RED, 37 focused GREEN checks, then unchanged v3 CPU."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import re
import tomllib

import verify_checkpoint_update as checkpoint
import verify_inbound_retry as retry

INPUTS = Path(__file__).with_name("receive_cache_inputs.json")
PREFIX = "protocol_engine::tests::failed_receive_cache_"
RED_TEST = PREFIX + "unchanged_failures_bind_envelope_and_event_context"
FAILURE = "inbound_receive_cache_tests.rs:69:5"
TEST_PATH = "chat-protocol/src/protocol_engine/inbound_receive_cache_tests.rs"
FIXTURES = {
    TEST_PATH: "3455ed4a62712756a2b31b1e9756a37582a2311ea3bd866f5e196719af8dd765",
    "chat-protocol/src/protocol_engine/batched_retry_cpu_tests.rs": "27e49662a218d2adba639b85b7274c0e679a6234a7ff2a37345da3dae7c3383a",
    "chat-protocol/src/protocol_engine/inbound_retry_backlog_tests.rs": "6d6796dfe3e49130667e4518e45010a4cee62e163761ea3f8f4a05c101d56f36",
    "chat-protocol/src/protocol_engine/inbound_retry_ciphertext_fixture_tests.rs": "4e865795edddef4371b8eed0f089b40a4ef87217ed120dc2c70bdcbdb2c88f6a",
}
GREEN_PATHS = ["chat-protocol/src/protocol_engine.rs", "chat-protocol/src/protocol_engine/engine_core.rs",
    "chat-protocol/src/protocol_engine/engine_incoming_retry.rs", "chat-protocol/src/protocol_engine/inbound_receive_cache.rs",
    "chat-protocol/src/protocol_engine/types.rs"]
PIN_PATHS = ["chat-protocol/Cargo.lock", "chat-protocol/Cargo.toml", "core/Cargo.lock", "core/Cargo.toml",
    "linux/Cargo.lock", "protocol-ffi/Cargo.lock"]
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
NDR = "nostr-double-ratchet"
CODEC = "nostr-double-ratchet-pairwise-codec"
NDR_CHECKSUMS = {"before": "8e67461f21af04a9675cd04419c1fe1eec5bd6035c203592bb2724d7d6f627cf",
    "after": "d838e57c5db5431ccc1e412ea795fed4013e288b7d139c39f29c1c9a2806a3e7"}
CODEC_CHECKSUM = "fc6dc58e28d3046422909bdfdc596cef9d6fc243a94657dc49767b07f0d7060b"


def focused_tests():
    return checkpoint.focused_tests() + [checkpoint.spec(PREFIX, 4, False),
        checkpoint.spec("protocol_engine::tests::handshake_proof_delivers_out_of_order_messages_and_survives_restart")]


def contract():
    data = json.loads(INPUTS.read_text())
    assert data["enabled"] is True, "Receive cache qualification disabled pending final source review"
    assert data["schema"] == "iris-receive-cache-input/v1"
    chain = data["source_chain"]
    refs = [data["base"], *[item["commit"] for item in chain]]
    assert len(chain) == 4 and len(set(refs)) == 5
    assert all(re.fullmatch(r"[0-9a-f]{40}", ref or "") for ref in refs)
    assert data["base"] == "ebfa822e08d95efad66b68288b3e66d1499b0f85"
    assert [item["commit"] for item in chain[:3]] == ["5c70faf9e84f9bfa2ae3439315620f3ea25eceac",
        "376374936bb365f0f691c8b3a74a8bd338a23568", "820f93d4edaefa01d45e2b032acd2edec0230056"]
    assert [item["parent"] for item in chain] == refs[:-1]
    assert (data["before"], data["after"]) == (chain[0]["commit"], chain[-1]["commit"])
    assert [item["changed_paths"] for item in chain[:3]] == [[TEST_PATH], GREEN_PATHS, ["RELEASE_NOTES.md"]]
    paths = chain[-1]["changed_paths"]
    assert paths == PIN_PATHS
    assert data["fixtures"] == FIXTURES
    assert data["before_test"] == RED_TEST and data["before_failure_marker"] == FAILURE
    assert data["before_observation_test"] is None and data["before_observation_failure_marker"] is None
    assert data["tests"] == focused_tests(), "Reviewed 37-test selection changed"
    assert data["cpu"] == checkpoint.cpu_plan(), "Unchanged v3 CPU plan required"
    assert set(data["dependencies"]) == {"before", "after"}
    for which, expected in data["dependencies"].items():
        assert set(expected) == {"manifest_sha256", "lock_sha256"}
        assert all(re.fullmatch(r"[0-9a-f]{64}", value or "") for value in expected.values())
    assert data["dependencies"]["before"] != data["dependencies"]["after"]
    return data


def dependencies(root, which, data):
    manifest_bytes = (root / "chat-protocol/Cargo.toml").read_bytes()
    lock_bytes = (root / "chat-protocol/Cargo.lock").read_bytes()
    hashes = {"manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "lock_sha256": hashlib.sha256(lock_bytes).hexdigest()}
    assert hashes == data["dependencies"][which], "Unreviewed manifest or locked dependencies"
    manifest, lock = tomllib.loads(manifest_bytes.decode()), tomllib.loads(lock_bytes.decode())
    version = "0.0.172" if which == "before" else "0.0.173"
    assert manifest["dependencies"][NDR] == version
    assert manifest["dependencies"][CODEC] == "0.0.168"
    packages = [p for p in lock["package"] if p["name"].startswith(NDR)]
    assert len(packages) == 2 and {p["name"] for p in packages} == {NDR, CODEC}
    for package in packages:
        assert package["source"] == REGISTRY
        assert package["version"] == (version if package["name"] == NDR else "0.0.168")
        assert package["checksum"] == (NDR_CHECKSUMS[which] if package["name"] == NDR else CODEC_CHECKSUM)
    return {**hashes, "packages": {p["name"]: {key: p[key] for key in ("version", "source", "checksum")} for p in packages}}


def source_proof(which, root, data):
    chain = data["source_chain"][:1] if which == "before" else data["source_chain"]
    proof = []
    for item in chain:
        commit, parent = item["commit"], item["parent"]
        assert retry.git(root, "rev-list", "--parents", "-n", "1", commit).split() == [commit, parent]
        changed = retry.git(root, "diff", "--name-only", parent, commit).splitlines()
        assert sorted(changed) == sorted(item["changed_paths"]), "Source exceeds reviewed per-commit scope"
        for path, digest in FIXTURES.items():
            actual = hashlib.sha256(checkpoint.source_bytes(root, commit, path)).hexdigest()
            assert actual == digest, "Cache regression or CPU fixture changed"
        proof.append({**item, "tree": retry.git(root, "rev-parse", commit + "^{tree}"),
            "changed_file_sha256": {path: hashlib.sha256(checkpoint.source_bytes(root, commit, path)).hexdigest() for path in changed}})
    tip = chain[-1]
    assert retry.git(root, "rev-parse", "HEAD") == tip["commit"]
    assert retry.git(root, "rev-parse", "HEAD^") == tip["parent"]
    for path, digest in FIXTURES.items():
        assert retry.sha(root / path) == digest
    dep = dependencies(root, which, data)
    return {"commit": tip["commit"], "parent": tip["parent"], "changed_paths": tip["changed_paths"],
        "tree": retry.git(root, "rev-parse", "HEAD^{tree}"), "protocol_tree": retry.git(root, "rev-parse", "HEAD:chat-protocol"),
        "fixtures": FIXTURES, "reviewed_chain": proof, "dependencies": dep, "protocol_lock_sha256": dep["lock_sha256"]}


def dependency_transition(before, after, data):
    for which, source in (("before", before), ("after", after)):
        assert source["commit"] == data[which]
        dep = source["dependencies"]
        assert {key: dep[key] for key in ("manifest_sha256", "lock_sha256")} == data["dependencies"][which]
        assert source["protocol_lock_sha256"] == dep["lock_sha256"]
        assert dep["packages"][NDR]["checksum"] == NDR_CHECKSUMS[which]
        assert dep["packages"][CODEC]["checksum"] == CODEC_CHECKSUM
    assert before["protocol_lock_sha256"] != after["protocol_lock_sha256"]
    return {"accepted": True, "before": before["dependencies"], "after": after["dependencies"],
        "fetch": "One bounded locked fetch per reviewed source lock, followed by offline release builds."}


def check_result(text, test, code, expected_failure=None):
    if test["name"] != RED_TEST:
        return checkpoint.check_result(text, test, code, expected_failure)
    counts = retry.check_result(text, test, code, expected_failure)
    assert expected_failure == FAILURE and text.count("panicked at") == 1
    assert "assertion `left == right` failed" in text
    rows = re.findall(r"(?:^|\n)\s*(left|right): (\([^\n]+\))", text)
    assert [key for key, _ in rows] == ["left", "right"]
    actual, expected = [ast.literal_eval(value) for _, value in rows]
    assert actual == (2, 2) and expected == (0, 1), "Require the intended final cache-hit assertion after positive controls"
    counts["receive_calls"] = {"observed_repeated": 2, "observed_changed_envelope": 2,
        "expected_repeated": 0, "expected_changed_envelope": 1}
    return counts


def run(which, root, out, data):
    retry.INPUTS = INPUTS
    if which == "preflight":
        retry.preflight(out, data)
    else:
        assert root is not None
        plan = {"tests": [checkpoint.spec(RED_TEST)]} if which == "before" else {"tests": data["tests"], "batched_cpu": data["cpu"]}
        retry.stage(which, root, out, data, release_build=True, result_checker=check_result,
            source_prover=source_proof, test_plan=plan, dependency_checker=dependency_transition)


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
