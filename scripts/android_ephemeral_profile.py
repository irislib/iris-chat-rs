#!/usr/bin/env python3
"""Prepare ordinary UI only in a user created by this test invocation."""

from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import time
import xml.etree.ElementTree as ET

SETUP_WIZARDS = ("com.google.android.setupwizard", "app.grapheneos.setupwizard")
SHADE_IDS = {"com.android.systemui:id/notification_panel", "com.android.systemui:id/notification_stack_scroller"}


@dataclass(frozen=True)
class OwnedEphemeralUser:
    user: int
    original_user: int
    preexisting_users: frozenset
    creation_response_sha256: str

    @classmethod
    def from_creation(cls, original_user, preexisting_users, creation_response):
        matches = re.findall(r"^Success: created user id (\d+)\s*$", creation_response, re.M)
        if len(matches) != 1 or type(original_user) is not int or original_user != 0:
            raise ValueError("Exact fresh-user creation and ordinary original user required")
        user = int(matches[0])
        if user == 0 or user in preexisting_users:
            raise ValueError("Refusing to prepare an existing Android user")
        return cls(user, original_user, frozenset(preexisting_users), hashlib.sha256(creation_response.encode()).hexdigest())

    def verify(self, adb):
        if (self.user <= 0 or self.original_user != 0 or self.user in self.preexisting_users
                or not re.fullmatch(r"[0-9a-f]{64}", self.creation_response_sha256)):
            raise ValueError("Fresh non-primary user ownership required")
        users = adb("shell", "pm", "list", "users")
        record = re.search(r"UserInfo\{" + str(self.user) + r":[^}\n]*:([0-9a-fA-F]+)\}", users)
        if record is None or not int(record.group(1), 16) & 0x100:
            raise ValueError("Owned user is not ephemeral")
        if adb("shell", "am", "get-current-user").strip() != str(self.user):
            raise ValueError("Owned ephemeral user must be current")


def foreground_package(xml):
    if not xml.strip() or len(xml) > 1024 * 1024:
        raise ValueError("Missing or oversized active UI hierarchy")
    root = ET.fromstring(xml)
    nodes = list(root.iter("node"))
    if not nodes or not nodes[0].get("package"):
        raise ValueError("Empty active UI hierarchy")
    if any(n.get("package") in SETUP_WIZARDS for n in nodes):
        raise ValueError("Fresh-profile setup wizard still covers ordinary UI")
    return nodes[0].get("package"), nodes


def unlocked(policy):
    if policy.count("KeyguardServiceDelegate") != 1:
        raise ValueError("Unknown keyguard state")
    delegate = policy.split("KeyguardServiceDelegate", 1)[1]
    if (not re.search(r"^\s*showing=false\s*$", delegate, re.M)
            or not re.search(r"^\s*inputRestricted=false\s*$", delegate, re.M)
            or "interactiveState=INTERACTIVE_STATE_AWAKE" not in delegate):
        raise ValueError("Ordinary UI is locked or asleep")


def verify_owned_ephemeral_ui(adb, ownership, output, capture_png):
    """Prove real HOME and notification shade, then leave HOME before testing."""
    ownership.verify(adb)
    output = Path(output)
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = {"user": ownership.user, "creation_response_sha256": ownership.creation_response_sha256,
              "passed": False, "screens": {}}
    home = None

    def ui(*args):
        ownership.verify(adb)
        return adb("shell", *args)

    def capture(stage):
        remote = f"/sdcard/iris-owned-profile-{ownership.user}.xml"
        ui("uiautomator", "dump", remote)
        xml = ui("cat", remote)
        (output / (stage + ".xml")).write_text(xml)
        ownership.verify(adb)
        png = capture_png()
        if not isinstance(png, bytes) or not png.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("Missing actual profile UI screenshot")
        (output / (stage + ".png")).write_bytes(png)
        report["screens"][stage] = {"xml_sha256": hashlib.sha256(xml.encode()).hexdigest(),
                                    "png_sha256": hashlib.sha256(png).hexdigest()}
        package, nodes = foreground_package(xml)
        report["screens"][stage].update(foreground_package=package, node_count=len(nodes))
        return package, nodes

    try:
        resolved = ui("cmd", "package", "resolve-activity", "--brief", "--user", str(ownership.user),
                      "-a", "android.intent.action.MAIN", "-c", "android.intent.category.HOME")
        components = re.findall(r"^([A-Za-z][\w.]*/[A-Za-z0-9_.$]+)\s*$", resolved, re.M)
        if len(components) != 1:
            raise ValueError("Installed ordinary HOME did not resolve uniquely")
        home = components[0].split("/", 1)[0]
        if home in (*SETUP_WIZARDS, "android", "com.android.systemui"):
            raise ValueError("Resolved HOME is not an ordinary launcher")
        if not ui("pm", "path", "--user", str(ownership.user), home).startswith("package:"):
            raise ValueError("Resolved HOME is not installed in the owned user")
        report["resolved_home"] = components[0]
        ui("input", "keyevent", "WAKEUP")
        ui("wm", "dismiss-keyguard")
        ui("input", "keyevent", "HOME")
        unlocked(ui("dumpsys", "window", "policy"))
        if capture("home")[0] != home:
            raise ValueError("Resolved ordinary HOME is not the active UI")
        ui("cmd", "statusbar", "expand-notifications")
        unlocked(ui("dumpsys", "window", "policy"))
        package, nodes = capture("shade")
        if package != "com.android.systemui" or not any(n.get("resource-id") in SHADE_IDS for n in nodes):
            raise ValueError("Actual SystemUI notification shade is unavailable")
        report["passed"] = True
    finally:
        # These UI actions are safe only while this invocation's user is current.
        try:
            ui("cmd", "statusbar", "collapse")
            ui("input", "keyevent", "HOME")
            if report["passed"]:
                unlocked(ui("dumpsys", "window", "policy"))
                if capture("restored-home")[0] != home:
                    raise ValueError("Ordinary HOME was not restored after shade verification")
            report["shade_collapsed"] = True
        except Exception:
            report["passed"] = False
            report["shade_collapsed"] = False
            raise
        finally:
            (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


WIZARD_STAGES = (
    ("welcome", "app.grapheneos.setupwizard", "Welcome to GrapheneOS", "Next"),
    ("location", "app.grapheneos.setupwizard", "Location services", "Next"),
    ("empty-pin", "com.android.settings", "Set a PIN", "Skip"),
    ("confirm-skip-pin", "com.android.settings", "Skip setup for PIN • Fingerprint?", "Skip"),
    ("migration", "app.grapheneos.setupwizard", "Restore apps & data", "Skip"),
    ("finish", "app.grapheneos.setupwizard", "You're all set now!", "Start"),
)


def wizard_control(xml, stage):
    """Accept only the observed secondary-user pages and one navigation control."""
    if not xml.strip() or len(xml) > 1024 * 1024:
        raise ValueError("Missing or oversized setup UI hierarchy")
    nodes = list(ET.fromstring(xml).iter("node"))
    name, package, title, label = WIZARD_STAGES[stage]
    labels = [n.get("text", "") for n in nodes]
    if not nodes or nodes[0].get("package") != package or labels.count(title) != 1:
        raise ValueError("Unrecognized or out-of-order owned-profile setup page: " + name)
    if any(word in "\n".join(labels).lower() for word in
           ("administrator", "organization", "work profile", "sign in", "add account")):
        raise ValueError("Account or administration setup is not allowed")
    inputs = [n for n in nodes if n.get("class", "").endswith("EditText")]
    if name == "empty-pin":
        if (len(inputs) != 1 or inputs[0].get("resource-id") != "com.android.settings:id/password_entry"
                or inputs[0].get("text") != "" or inputs[0].get("package") != package
                or inputs[0].get("class") != "android.widget.EditText"
                or not any(t.startswith("PIN must be at least 4 digits") for t in labels)):
            raise ValueError("Only the known empty optional PIN-creation page may be skipped")
    elif inputs:
        raise ValueError("Unexpected credential or text-entry setup page")
    if name == "confirm-skip-pin" and ("Cancel" not in labels or not any(
            t.startswith("A PIN is required to set up Fingerprint Unlock.") for t in labels)):
        raise ValueError("Unknown optional PIN-skip confirmation")
    controls = [n for n in nodes if n.get("package") == package and n.get("text") == label
                and n.get("enabled") == "true" and n.get("clickable") == "true"
                and n.get("class") == "android.widget.Button"]
    if len(controls) != 1:
        raise ValueError("Normal setup control is missing or ambiguous")
    bounds = controls[0].get("bounds", "")
    match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", bounds)
    if match is None:
        raise ValueError("Invalid setup control bounds")
    x1, y1, x2, y2 = map(int, match.groups())
    if x2 <= x1 or y2 <= y1:
        raise ValueError("Empty setup control bounds")
    return {"stage": name, "package": package, "label": label, "bounds": bounds,
            "tap": ((x1 + x2) // 2, (y1 + y2) // 2)}


def prepare_owned_ephemeral_profile(adb, ownership, output, capture_png):
    """Use normal visible setup only; the wizard sets its own completion state."""
    ownership.verify(adb)
    output = Path(output)
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    wizard_dir = output / "wizard"
    wizard_dir.mkdir(mode=0o700)
    deadline = time.monotonic() + 300
    report = {"passed": False, "user": ownership.user,
              "creation_response_sha256": ownership.creation_response_sha256,
              "wizard": {"actions": [], "screens": {}, "observed_user_setup_complete": False},
              "restoration": "Original outer controller removes this exact freshly created ephemeral user"}

    def ui(*parts):
        ownership.verify(adb)
        if time.monotonic() > deadline:
            raise ValueError("Owned-profile normal setup exceeded its bounded deadline")
        return adb("shell", *parts)

    def capture(stage):
        remote = f"/sdcard/iris-owned-wizard-{ownership.user}.xml"
        ui("uiautomator", "dump", remote)
        xml = ui("cat", remote)
        (wizard_dir / (stage + ".xml")).write_text(xml)
        ownership.verify(adb)
        png = capture_png()
        if not isinstance(png, bytes) or not png.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("Missing actual setup UI screenshot")
        (wizard_dir / (stage + ".png")).write_bytes(png)
        report["wizard"]["screens"][stage] = {"xml_sha256": hashlib.sha256(xml.encode()).hexdigest(),
                                                "png_sha256": hashlib.sha256(png).hexdigest()}
        return xml

    try:
        for stage, (name, _, _, _) in enumerate(WIZARD_STAGES):
            control = wizard_control(capture(name), stage)
            # Recheck the entire accepted page, including empty PIN, immediately before tapping.
            if wizard_control(capture(name + "-recheck"), stage) != control:
                raise ValueError("Normal setup control changed before action")
            ui("input", "tap", *map(str, control["tap"]))
            report["wizard"]["actions"].append(control)
        foreground_package(capture("completed"))
        if ui("settings", "--user", str(ownership.user), "get", "secure", "user_setup_complete").strip() != "1":
            raise ValueError("Wizard did not complete its own user setup")
        report["wizard"]["observed_user_setup_complete"] = True
        report.update(verify_owned_ephemeral_ui(adb, ownership, output / "ordinary-ui", capture_png))
    except Exception as error:
        report.update(passed=False, error=str(error))
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    return report
