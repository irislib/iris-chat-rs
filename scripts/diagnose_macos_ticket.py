#!/usr/bin/env python3
"""Read-only diagnostic: compare Apple's public ticket service and stapler.

Use an attested, already published app. Do not sign, submit, launch, or modify
it. Keep signing identifiers and raw tool output out of the diagnostic receipt.
"""
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


app = Path(sys.argv[1])
destination = Path(sys.argv[2])
assert app.is_dir()
destination.parent.mkdir(parents=True, exist_ok=True)


def run(argv, **kwargs):
    started = time.monotonic()
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=90, **kwargs)
        return result.returncode, result.stdout, result.stderr, round(time.monotonic() - started, 2)
    except subprocess.TimeoutExpired:
        return 124, "", "diagnostic process deadline exceeded", round(time.monotonic() - started, 2)


code, stdout, stderr, _ = run(["codesign", "-d", "--verbose=4", str(app)])
assert code == 0
details = stdout + stderr
match = re.search(r"^CDHash=([0-9a-f]{40})$", details, re.M)
assert match and "Hash type=sha256" in details
record = "2/2/" + match.group(1)
endpoint = "https://api.apple-cloudkit.com/database/1/com.apple.gk.ticket-delivery/production/public/records/lookup"
code, stdout, stderr, seconds = run(
    ["curl", "--silent", "--show-error", "--connect-timeout", "10", "--max-time", "45",
     "--header", "Content-Type: application/json", "--data-binary", "@-",
     "--write-out", "\n%{http_code}", endpoint],
    input=json.dumps({"records": [{"recordName": record}]}),
)
body, _, status = stdout.rpartition("\n")
lookup = {"exitCode": code, "httpStatus": int(status) if status.isdigit() else None, "seconds": seconds}
if code == 0 and status == "200":
    data = json.loads(body)
    records = data.get("records", [])
    lookup["signedTicketAvailable"] = any(
        row.get("recordName") == record
        and row.get("recordType") == "DeveloperIDTicket"
        and row.get("fields", {}).get("signedTicket", {}).get("value")
        for row in records
    )
    lookup["serverErrorCodes"] = [row.get("serverErrorCode") for row in records if row.get("serverErrorCode")]

attempts = []
for attempt in range(2):
    if attempt:
        time.sleep(30)
    code, stdout, stderr, seconds = run(["xcrun", "stapler", "validate", "--verbose", str(app)])
    output = stdout + stderr
    attempts.append({
        "attempt": attempt + 1,
        "exitCode": code,
        "seconds": seconds,
        "errorDomains": sorted(set(re.findall(r"Error Domain=([A-Za-z0-9_.]+)", output))),
        "errorCodes": sorted(set(re.findall(r"Code=(-?\d+)", output))),
        "httpStatuses": sorted(set(re.findall(r"Status Code: (\d+)", output))),
        "requestTimedOut": "timed out" in output.lower(),
        "secureConnectionFailed": "could not establish secure connection" in output.lower(),
        "certificateTrustFailed": "certificate trust evaluation did not return expected result" in output.lower(),
        "recordNotFound": "record not found" in output.lower(),
        "unexpectedServiceResponse": "response is inconsistent" in output.lower(),
    })
    if code == 0:
        break

receipt = {
    "diagnosticOnly": True,
    "releaseGate": False,
    "baselineTag": "v2026.10.13",
    "runner": "macos-15",
    "readOnly": True,
    "checkedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "lookup": lookup,
    "stapler": attempts,
}
event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text()) if os.environ.get("GITHUB_EVENT_PATH") else {}
extra_hashes = json.loads(event.get("inputs", {}).get("notary_code_hashes", "[]"))
assert isinstance(extra_hashes, list) and len(extra_hashes) <= 4
extra_results = []
for extra_hash in extra_hashes:
    assert isinstance(extra_hash, str) and re.fullmatch(r"[0-9a-f]{40}", extra_hash)
    print("::add-mask::" + extra_hash, flush=True)
    request = json.dumps({"records": [{"recordName": "2/2/" + extra_hash}]})
    code, stdout, stderr, seconds = run(
        ["curl", "--silent", "--show-error", "--connect-timeout", "10", "--max-time", "45",
         "--header", "Content-Type: application/json", "--data-binary", "@-",
         "--write-out", "\n%{http_code}", endpoint], input=request,
    )
    body, _, status = stdout.rpartition("\n")
    lookup = {"exitCode": code, "httpStatus": int(status) if status.isdigit() else None, "seconds": seconds}
    if code == 0 and status == "200":
        records = json.loads(body).get("records", [])
        lookup["signedTicketAvailable"] = any(row.get("fields", {}).get("signedTicket", {}).get("value") for row in records)
        lookup["serverErrorCodes"] = [row.get("serverErrorCode") for row in records if row.get("serverErrorCode")]
    swift_code, swift_stdout, swift_stderr, swift_seconds = run(
        ["swift", str(Path(__file__).with_suffix(".swift"))], input=request,
    )
    foundation = {"exitCode": swift_code, "seconds": swift_seconds}
    if swift_code == 0:
        foundation["response"] = json.loads(swift_stdout)
    extra_results.append({"curl": lookup, "foundation": foundation})
receipt["additionalTicketLookups"] = extra_results
destination.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt))
raise SystemExit(0 if attempts[-1]["exitCode"] == 0 and all(x["foundation"]["exitCode"] == 0 for x in extra_results) else 1)
