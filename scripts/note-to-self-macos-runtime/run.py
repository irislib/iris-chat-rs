"""Run only on two expressly reserved SSH-accessible synthetic macOS desktop sessions."""
import argparse
import base64
from contextlib import ExitStack
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import subprocess
import time
import uuid

from evidence import REPO, compare_copies, product_identity, product_spec, require, sha256

FILES = ("actor.py", "mac_ui.js", "evidence.py")
BOOT = """
import json,os,pathlib,runpy,sys,tempfile
os.umask(0o077)
with tempfile.TemporaryDirectory(prefix='notes-driver-') as tmp:
    code = json.loads(sys.stdin.buffer.readline())
    for name in ('evidence.py','mac_ui.js','actor.py'):
        pathlib.Path(tmp,name).write_text(code[name])
    sys.path.insert(0,tmp)
    runpy.run_path(str(pathlib.Path(tmp,'actor.py')),run_name="__main__")
"""


class Peer:
    def __init__(self, host, python):
        require(bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9@.:-]*", host)), "Invalid SSH alias")
        require(re.fullmatch(r"[A-Za-z0-9./_-]+", python) is not None, "Invalid Python executable")
        self.proc = subprocess.Popen(
            ["ssh", "-T", "-oBatchMode=yes", "-oStrictHostKeyChecking=yes", "-oConnectTimeout=8",
             "-oForwardAgent=no", "-oClearAllForwardings=yes", host, shlex.join([python, "-u", "-c", BOOT])],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )

    def __enter__(self):
        here = Path(__file__).parent
        self.write({name: (here / name).read_text() for name in FILES})
        return self

    def write(self, value):
        self.proc.stdin.write(json.dumps(value) + "\n")
        self.proc.stdin.flush()

    def call(self, action, *args):
        self.write([action, args])
        with selectors.DefaultSelector() as events:
            events.register(self.proc.stdout, selectors.EVENT_READ)
            require(events.select(300), "Timed out waiting for remote " + action)
            line = self.proc.stdout.readline()
        require(line, "Remote worker ended during " + action)
        result = json.loads(line)
        require(result.get("ok"), result.get("error", "Unknown worker failure"))
        return result["result"]

    def __exit__(self, *exc):
        self.proc.stdin.close()
        try:
            rc = self.proc.wait(timeout=30)
            if not exc[0]:
                require(rc == 0, "Remote worker cleanup did not finish")
        except subprocess.TimeoutExpired:
            self.proc.terminate()
            self.proc.wait(timeout=10)
            raise


def api(route):
    result = subprocess.run(["gh", "api", f"repos/{REPO}/{route}"], capture_output=True,
                            text=True, timeout=40, check=True)
    return json.loads(result.stdout)


def exact_product(source, run_id, artifact_id):
    product_identity(source, run_id, artifact_id)
    run = api(f"actions/runs/{run_id}")
    artifact = api(f"actions/artifacts/{artifact_id}")
    commit = api("git/commits/" + source)
    require(commit["sha"] == source, "Product commit lookup returned another source")
    return product_spec(run, artifact, commit["tree"]["sha"],
                        source=source, run_id=run_id, artifact_id=artifact_id)


def exercise(sender, receiver, spec, options, trace, run, settle=time.sleep):
    systems = [sender.call("preflight", options.sender_user, run),
               receiver.call("preflight", options.receiver_user, run)]
    require(len(set(systems)) == 2, "Two independent booted OS instances are required")
    products = [sender.call("bind", options.sender_archive, spec, options.reserve_gib),
                receiver.call("bind", options.receiver_archive, spec, options.reserve_gib)]
    require(products[0] == products[1], "The actors did not bind identical app and CLI bytes")
    trace.update(instances=systems, binaries=products[0], artifact=spec)
    owner = receiver.call("create")
    first = receiver.call("approve", sender.call("request", "main"))
    laptop = sender.call("authorized", "main")
    require(owner["owner"] == laptop["owner"] and set(first) == {owner["device"], laptop["device"]}
            and len(first) == 2, "Expected two distinct authorized devices on one account")
    # C has a pending link request. Its own service is confirmed stopped before
    # B approves it. C cannot publish a personal messaging invite until resumed.
    devices = receiver.call("approve", receiver.call("request", "late"))
    new = set(devices) - set(first)
    require(len(devices) == 3 and len(new) == 1 and set(first) < set(devices), "Third device not enrolled")
    late = new.pop()
    sender.call("launch", devices)
    receiver.call("launch", devices)
    payload = sender.call("send")
    def offer(peer, name="main"):
        return peer.call("offer", owner["owner"], laptop["device"], name)
    original = offer(sender)
    compare_copies(original, [offer(receiver)], payload)
    retry_args = [owner["owner"], laptop["device"], owner["device"], late, original["message"]]
    sender.call("retry", *retry_args, "receiver-complete")
    result = receiver.call("accept", original, options.screenshots)
    digest = result["sha256"]
    if options.screenshots:
        require(set(result["images"]) == {"offered", "completed"}, "Missing required window captures")
        for stage, encoded in result["images"].items():
            image = base64.b64decode(encoded, validate=True)
            require(image.startswith(b"\x89PNG\r\n\x1a\n"), "Invalid screenshot")
            with options.receipt.with_name(options.receipt.name + "." + stage + ".png").open("xb") as file:
                file.write(image)
    require(digest == payload["sha256"], "Receiver saved different bytes")
    trace.update(offer=original, file=payload, acceptedSHA256=digest, pendingMissingCopy=True)
    require(receiver.call("resume") == {"owner": owner["owner"], "device": late},
            "The resumed device is not the originally enrolled sibling")
    compare_copies(original, [offer(receiver, "late")], payload)
    sender.call("retry", *retry_args, "drained")
    # Recheck independently after C has actually received its copy.
    for _ in range(3):
        settle(3)
        compare_copies(original, [offer(sender), offer(receiver), offer(receiver, "late")], payload)
        require(sender.call("check_live", original, False) is True
                and receiver.call("check_live", original, True) == digest, "Live app or accepted file changed")
    trace.update(lateMatched=True, retryDrained=True, copiesPerDevice=[1, 1, 1], bothAppsStayedOpen=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--product-source", required=True, help="Reviewed full source commit of the test product")
    parser.add_argument("--product-run", required=True, type=int, help="Exact successful producer run ID")
    parser.add_argument("--product-artifact", required=True, type=int, help="Exact immutable product artifact ID")
    for role in ["sender", "receiver"]:
        parser.add_argument("--"+role, required=True, help="SSH alias for assigned GUI session")
        parser.add_argument("--"+role+"-user", required=True, help="Explicitly reserved synthetic GUI login")
        parser.add_argument("--"+role+"-archive", required=True, help="Exact artifact ZIP already staged on that OS")
    parser.add_argument("--python", default="python3.13", help="Remote Python 3.12+ executable")
    parser.add_argument("--reserve-gib", required=True, type=int, help="Per-host configured free-space floor in GiB")
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--screenshots", action="store_true",
                        help="Privately capture only the receiver app window; requires Screen Recording")
    parser.add_argument("--reserved-test-desktops", required=True, action="store_true",
                        help="Both independent graphical logins are expressly reserved for this run")
    options = parser.parse_args()
    require(options.reserve_gib > 0, "A positive free-space floor is required")
    os.umask(0o077)
    product_identity(options.product_source, options.product_run, options.product_artifact)
    here = Path(__file__).parent
    trace = {"passed": False, "kind": "two-mac-runtime", "source": options.product_source,
             "ciRun": options.product_run,
             "driverSHA256": {name: sha256(here / name) for name in (*FILES, "run.py")}}
    with options.receipt.open("x") as output:
        try:
            spec = exact_product(options.product_source, options.product_run, options.product_artifact)
            with ExitStack() as stack:
                sender = stack.enter_context(Peer(options.sender, options.python))
                receiver = stack.enter_context(Peer(options.receiver, options.python))
                exercise(sender, receiver, spec, options, trace, uuid.uuid4().hex)
            trace["passed"] = True
        except Exception as e:
            trace["error"] = str(e) if isinstance(e, RuntimeError) else type(e).__name__
            raise
        finally:
            json.dump(trace, output, indent=2)
            output.write("\n")


if __name__ == "__main__":
    main()
