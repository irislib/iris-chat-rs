#!/usr/bin/env python3
"""Exercise real first-contact messaging with two disposable CLI identities."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
from urllib.parse import urlparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iris", default="iris", help="Installed iris executable")
    parser.add_argument("--relay", required=True, help="Chosen ws:// or wss:// message server")
    parser.add_argument("--timeout", type=int, default=180, help="Total demo deadline in seconds")
    args = parser.parse_args()
    executable = shutil.which(args.iris)
    if not executable:
        parser.error("iris is not installed or the executable cannot be found")
    relay = urlparse(args.relay)
    if relay.scheme not in ("ws", "wss") or not relay.hostname or relay.username or relay.password:
        parser.error("--relay must be a ws:// or wss:// URL without credentials")
    if args.timeout < 30:
        parser.error("--timeout must be at least 30 seconds")

    started = time.monotonic()
    deadline = started + args.timeout
    processes = []
    files = []
    current_step = "startup"

    def interrupt(_signum, _frame):
        raise InterruptedError("demo interrupted")

    signal.signal(signal.SIGTERM, interrupt)

    def remaining():
        seconds = deadline - time.monotonic()
        if seconds <= 0:
            raise TimeoutError("demo deadline reached")
        return min(45, seconds)

    def report(step, **fields):
        nonlocal current_step
        current_step = step
        print(json.dumps({"step": step, "elapsed_seconds": round(time.monotonic() - started, 2), **fields}), flush=True)

    with tempfile.TemporaryDirectory(prefix="iris-agent-demo-") as temporary:
        root = Path(temporary)
        os.chmod(root, 0o700)
        config = root / "hashtree-config"
        config.mkdir()
        (config / "config.toml").write_text(
            '[blossom]\nservers = []\nread_servers = ["http://127.0.0.1:9"]\n'
            'write_servers = ["http://127.0.0.1:9"]\n'
        )
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("IRIS_", "HTREE_")) and key != "NOSTR_RELAYS"}
        env.update({
            "IRIS_DEMO_RELAYS": "",
            "IRIS_FIPS_WEBSOCKET_SEED_URLS": "",
            "IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR": "",
            "IRIS_CHAT_FIPS_UDP_BIND_ADDR": "",
            "IRIS_CHAT_FIPS_ENABLE_WEBRTC": "0",
            "IRIS_CHAT_FIPS_STATIC_PEERS": "",
            "IRIS_CHAT_FIPS_ROUTED_PEERS": "",
            "IRIS_CHAT_FIPS_TRUSTED_RATERS": "",
            "IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR": "",
            "IRIS_CHAT_SAME_HOST_HASHTREE": "0",
            "IRIS_UPDATE_RELAYS": "",
            "IRIS_UPDATE_BLOSSOM_SERVERS": "http://127.0.0.1:9",
            "HTREE_CONFIG_DIR": str(config),
        })

        def command(profile, *command_args):
            return [executable, "--json", "--no-background-sync", "--data-dir",
                    str(root / profile), *command_args]

        def environment(profile):
            return {**env, "HTREE_DATA_DIR": str(root / profile / "hashtree-data"),
                    "XDG_DATA_HOME": str(root / profile / "xdg-data")}

        def run(profile, *command_args):
            # Never include command arguments or raw outputs in diagnostics: invites contain secrets.
            result = subprocess.run(command(profile, *command_args), env=environment(profile),
                                    capture_output=True, timeout=remaining())
            if result.returncode:
                raise RuntimeError("CLI command failed (exit %d)" % result.returncode)
            response = json.loads(result.stdout)
            if response.get("status") != "ok":
                raise RuntimeError("CLI returned a non-success response")
            return response["data"]

        def start(profile, *command_args):
            output_path = root / (profile + "-" + command_args[0] + ".jsonl")
            output = output_path.open("wb")
            files.append(output)
            process = subprocess.Popen(command(profile, *command_args), env=environment(profile),
                                       stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.DEVNULL)
            processes.append(process)
            while True:
                remaining()
                if process.poll() is not None:
                    raise RuntimeError("owned CLI process exited before readiness")
                rows = read_stream(output_path)
                if any(row.get("data", {}).get("ready") for row in rows):
                    return output_path
                time.sleep(0.1)

        def read_stream(path):
            rows = []
            for line in path.read_text().splitlines():
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    pass  # The writer may still be completing its last line.
            return rows

        def wait_message(profile, peer, body, delivery=None):
            while True:
                remaining()
                rows = run(profile, "read", peer, "--limit", "20")["messages"]
                for row in rows:
                    if row["body"] == body and (delivery is None or row["delivery"] in delivery):
                        return row
                time.sleep(0.2)

        try:
            version = subprocess.check_output([executable, "--version"], text=True, timeout=10).strip()
            report("create_profiles", version=version)
            for profile in ("human", "agent"):
                start(profile, "service", "run")
                run(profile, "relay", "set", args.relay)
                run(profile, "account", "create", "--name", "Demo " + profile)
                state = run(profile, "debug")
                if state["relay_urls"] != [args.relay] or state["fips_transport"]["status"] != "unavailable":
                    raise RuntimeError("demo transport isolation was not established")
            human_id = run("human", "whoami")["user_id"]
            agent_id = run("agent", "whoami")["user_id"]
            if human_id == agent_id:
                raise RuntimeError("expected independent identities")
            report("accept_invite")
            invite = run("agent", "invite", "create", "--full")["url"]
            chat = run("human", "invite", "accept", invite)["current_chat"]["chat_id"]
            stream = start("agent", "listen")
            request = "Please reply to this Iris onboarding demo."
            first_send = run("human", "send", chat, request)
            incoming = wait_message("agent", human_id, request)
            sent = wait_message("human", chat, request)
            if sent["delivery"] == "seen":
                raise RuntimeError("reading or listening marked the request seen")
            while not any(row.get("command") == "message"
                          and row.get("data", {}).get("id") == incoming["id"]
                          and row["data"].get("is_outgoing") is False for row in read_stream(stream)):
                remaining()
                time.sleep(0.1)
            report("request_received", initial_delivery=first_send["delivery"],
                   listener_observed=True, read_did_not_mark_seen=True)
            # The invite's recipient is a new message request on the inviter's side.
            # Replying accepts the conversation; merely reading must not do so.
            reply = "Received. This reply came from a separate agent identity."
            run("agent", "send", human_id, reply)
            wait_message("human", chat, reply)
            wait_message("agent", human_id, reply, {"received", "seen"})
            run("agent", "seen", human_id, incoming["id"])
            wait_message("human", chat, request, {"seen"})
            report("passed", checks=["independent_identities", "full_invite", "listen",
                                     "bidirectional_delivery", "explicit_seen"])
            return 0
        except (OSError, RuntimeError, ValueError, KeyError, TypeError, KeyboardInterrupt,
                subprocess.SubprocessError) as error:
            report("failed", during=current_step, error=type(error).__name__,
                   detail="Check the chosen server and installed CLI; no send is retried automatically.")
            return 1
        finally:
            # Stop only children created above, then discard only their disposable profiles.
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
            for process in reversed(processes):
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            for output in files:
                output.close()


if __name__ == "__main__":
    raise SystemExit(main())
