#!/usr/bin/env python3
"""Finish Zapstore metadata and require public persistence before release success."""

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

AUTHOR = "7118857500bfa007934147623e9075a72e37a514e605524911d0f6caace41b07"
PACKAGE = "to.iris.chat"
RELAY = "wss://relay.zapstore.dev"
CDN = "https://cdn.zapstore.dev"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def tag(event, name):
    matches = [row[1] for row in event["tags"] if len(row) >= 2 and row[0] == name]
    require(len(matches) == 1, "Expected exactly one " + name + " tag")
    return matches[0]


def validate_chain(app, release, apk, expected):
    for event, kind in ((app, 32267), (release, 30063), (apk, 3063)):
        require(event["kind"] == kind and event["pubkey"] == AUTHOR, "Wrong event kind or publisher")
    require(tag(app, "d") == PACKAGE and tag(app, "name") == "Iris Chat", "Wrong app")
    require(tag(release, "i") == tag(apk, "i") == PACKAGE, "Wrong package")
    require(tag(release, "version") == tag(apk, "version") == expected["version"], "Wrong release version")
    require(tag(release, "d") == PACKAGE + "@" + expected["version"], "Wrong release address")
    require(tag(release, "c") == "main" and tag(release, "e") == apk["id"], "Wrong channel or APK reference")
    require(tag(release, "f") == tag(apk, "f") == "android-arm64-v8a", "Wrong platform")
    require(tag(apk, "x") == expected["sha256"] and tag(apk, "size") == str(expected["size"]), "Wrong APK bytes")
    require(tag(apk, "version_code") == expected["version_code"] and tag(apk, "filename") == expected["name"], "Wrong APK identity")
    require(tag(apk, "url") == CDN + "/" + expected["sha256"], "Unexpected APK URL")
    require(tag(apk, "m") == "application/vnd.android.package-archive", "Wrong APK MIME type")
    require(re.fullmatch(r"[0-9a-f]{64}", tag(apk, "apk_certificate_hash")), "Invalid APK certificate")


def verify_cdn(expected):
    url = CDN + "/" + expected["sha256"]
    destination = "https://zsapk.b-cdn.net/blobs/" + expected["sha256"] + ".apk"

    class Redirect(HTTPRedirectHandler):
        def redirect_request(self, request, fp, code, message, headers, new_url):
            require(request.full_url == url and new_url == destination, "Unexpected APK redirect")
            return super().redirect_request(request, fp, code, message, headers, new_url)

    digest, count, started = hashlib.sha256(), 0, time.monotonic()
    with build_opener(ProxyHandler({}), Redirect()).open(Request(url), timeout=20) as response:
        require(response.status == 200, "APK is not publicly available")
        while True:
            require(time.monotonic() - started < 90, "APK verification deadline exceeded")
            chunk = response.read(min(1048576, expected["size"] - count + 1))
            if not chunk:
                break
            count += len(chunk)
            require(count <= expected["size"], "Public APK exceeds expected size")
            digest.update(chunk)
    require(count == expected["size"] and digest.hexdigest() == expected["sha256"], "Public APK differs from immutable release")
    return {"bytes": count, "sha256": digest.hexdigest()}


def complete_publication(client, captured, expected, read_apk=verify_cdn):
    require(len(captured) == 3 and {e["kind"] for e in captured} == {32267, 30063, 3063}, "zsp did not return the three signed release events")
    app, release, apk = (next(e for e in captured if e["kind"] == kind) for kind in (32267, 30063, 3063))
    for event in captured:
        client.verify(event)
    validate_chain(app, release, apk, expected)

    certificate = tag(apk, "apk_certificate_hash")
    proofs = client.query(["--kind", "30509", "-d", certificate])
    require(proofs, "No public certificate ownership proof. Publish a C1 proof from the APK signing key before retrying.")
    # C1 chooses the latest proof; an older active proof cannot override a revocation.
    proof = sorted(proofs, key=lambda event: (-event["created_at"], event["id"]))[0]
    require(proof["kind"] == 30509 and proof["pubkey"] == AUTHOR and tag(proof, "d") == certificate, "Wrong certificate ownership proof")
    require(not any(t[0] == "revoked" for t in proof["tags"]), "Certificate ownership proof is revoked")
    require(int(tag(proof, "expiry")) > time.time(), "Certificate ownership proof expired; renew it before retrying")
    tag(proof, "signature")

    address = "32267:" + AUTHOR + ":" + PACKAGE
    references = [t for t in release["tags"] if t[0] == "a"]
    if references:
        require(tag(release, "a") == address, "Release references another app")
    else:
        unsigned = {k: copy.deepcopy(v) for k, v in release.items() if k not in ("id", "sig")}
        unsigned["tags"].insert(0, ["a", address])
        unsigned["created_at"] = max(int(time.time()), release["created_at"] + 1)
        release = client.sign(unsigned)
        require({k: v for k, v in release.items() if k not in ("id", "sig")} == unsigned, "Signing changed release fields")
        client.verify(release)

    events = [apk, release, app]
    client.publish(events)
    # A true OK can mean only "saved when complete". Only actual public events count.
    arguments = [argument for event in events for argument in ("--id", event["id"])]
    public = client.query(arguments)
    by_id = {event["id"]: event for event in public}
    require(set(by_id) == {event["id"] for event in events}, "Zapstore acknowledged publication but the complete release is not public")
    for event in events:
        require(by_id[event["id"]] == event, "Public event differs from the signed publication")
    validate_chain(by_id[app["id"]], by_id[release["id"]], by_id[apk["id"]], expected)
    require(tag(by_id[release["id"]], "a") == address, "Public release lacks its app reference")
    body = read_apk(expected)
    return {"passed": True, "event_ids": [e["id"] for e in events], "c1_event_id": proof["id"],
            "certificate": certificate, "apk": body}


class Tools:
    def __init__(self, evidence):
        self.evidence, self.sequence = evidence, 0
        self.secret = os.environ.get("SIGN_WITH", "")
        require(self.secret, "SIGN_WITH is required")
        self.environment = os.environ.copy()
        for key in ("SIGN_WITH", "NOSTR_SECRET_KEY", "NOSTR_CLIENT_KEY"):
            self.environment.pop(key, None)

    def run(self, command, data=None, signing=False, zsp=False):
        environment = self.environment.copy()
        if signing:
            environment["SIGN_WITH" if zsp else "NOSTR_SECRET_KEY"] = self.secret
        if zsp:
            environment.update(RELAY_URLS=RELAY, BLOSSOM_URL=CDN)
        result = subprocess.run(command, input=data, capture_output=True, env=environment, timeout=600 if zsp else 30)
        self.sequence += 1
        for suffix, value in (("stdout", result.stdout), ("stderr", result.stderr)):
            (self.evidence / f"{self.sequence:02d}-{command[0]}.{suffix}").write_bytes(value[:1048576].replace(self.secret.encode(), b"[REDACTED]"))
        require(all(self.secret.encode() not in value for value in (result.stdout, result.stderr)), "Unexpected signer secret in command output was redacted")
        require(result.returncode == 0, command[0] + " failed; retained output: " + str(self.evidence))
        require(len(result.stdout) <= 1048576 and len(result.stderr) <= 1048576, "Command output exceeded limit")
        return result.stdout

    def verify(self, event):
        self.run(["nak", "verify"], json.dumps(event).encode())

    def sign(self, event):
        return json.loads(self.run(["nak", "event"], json.dumps(event).encode(), signing=True))

    def query(self, arguments):
        raw = self.run(["nak", "req", "--auth=false", "--force-pre-auth=false", "--author", AUTHOR,
                        "--limit", "5", *arguments, RELAY])
        events = [json.loads(line) for line in raw.splitlines() if line.strip()]
        require(len(events) <= 5, "Too many relay results")
        for event in events:
            require(event["pubkey"] == AUTHOR, "Unexpected publisher in relay response")
            self.verify(event)
        return events

    def publish(self, events):
        self.run(["nak", "event", "--auth=false", RELAY],
                 b"".join(json.dumps(event).encode() + b"\n" for event in events))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    evidence = Path(tempfile.mkdtemp(prefix="iris-zapstore-"))
    receipt = {"passed": False, "tag": args.tag}
    try:
        manifest = json.loads(Path(args.manifest).read_text())
        require(manifest["tag"] == args.tag, "Manifest tag differs")
        name = "iris-chat-" + args.tag + "-android-arm64.apk"
        expected, = [dict(a) for a in manifest["assets"] if a["name"] == name]
        expected["version"] = manifest["version"]
        parts = [int(part) for part in manifest["version"].split(".")]
        require(len(parts) in (3, 4), "Unexpected release version")
        year, month, day, revision = parts + [0] * (4 - len(parts))
        expected["version_code"] = str(year * 1000000 + month * 10000 + day * 100 + revision)
        client = Tools(evidence)
        raw = client.run(["zsp", "publish", args.config, "--channel", "main", "--quiet", "--skip-preview",
                          "--overwrite-release", "--json"], signing=True, zsp=True)
        captured = [json.loads(line) for line in raw.splitlines() if line.strip()]
        receipt.update(complete_publication(client, captured, expected))
    except Exception as error:
        receipt["error"] = str(error)
    (evidence / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print("Zapstore public verification: " + str(evidence / "receipt.json"))
    if not receipt["passed"]:
        print(receipt["error"], file=sys.stderr)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
