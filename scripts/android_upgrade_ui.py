"""Drive the unchanged release APK through its visible Android accessibility UI."""
import json
from pathlib import Path
import re
import subprocess
import shutil
import time
import xml.etree.ElementTree as ET

PACKAGE = "to.iris.chat"


def bounds(node):
    match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
    if not match:
        raise ValueError("Missing UI bounds")
    left, top, right, bottom = map(int, match.groups())
    if right <= left or bottom <= top:
        raise ValueError("UI control has empty bounds")
    return left, top, right, bottom


def matching(root, *, text=None, description=None, kind=None, package=PACKAGE):
    return [node for node in root.iter("node") if node.get("package") == package
            and (text is None or node.get("text") == text)
            and (description is None or node.get("content-desc") == description)
            and (kind is None or node.get("class") == kind)]


def clickable(root, nodes):
    parents = {child: parent for parent in root.iter() for child in parent}
    controls = []
    for node in nodes:
        while node is not None and node.get("clickable") != "true":
            node = parents.get(node)
        if node is not None and node.get("enabled") == "true" and node not in controls:
            bounds(node)
            controls.append(node)
    if len(controls) != 1:
        raise ValueError("UI control is missing or ambiguous")
    return controls[0]


class Bridge:
    def __init__(self, adb, serial, output):
        if not re.fullmatch(r"emulator-\d+", serial):
            raise ValueError("Upgrade tests require an explicitly selected emulator")
        self.base = [str(adb), "-s", serial]
        self.output = Path(output)
        self.output.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.sequence = 0

    def run(self, *parts, binary=False, check=True, timeout=30):
        if shutil.disk_usage(self.output).free < 9.5 * 1024**3:
            raise RuntimeError("Upgrade test stopped at the host free-space reserve")
        self.sequence += 1
        result = subprocess.run([*self.base, *map(str, parts)], capture_output=True, timeout=timeout)
        with (self.output / "commands.jsonl").open("a") as log:
            log.write(json.dumps({"sequence": self.sequence, "args": list(map(str, parts)),
                                  "exit": result.returncode, "bytes": len(result.stdout)}) + "\n")
        if check and result.returncode:
            raise RuntimeError("Android command failed: " + result.stderr.decode(errors="replace")[:500])
        return result.stdout if binary else result.stdout.decode(errors="replace")

    def shell(self, *parts, **kwargs):
        return self.run("shell", *parts, **kwargs)


class UI:
    def __init__(self, bridge):
        self.bridge = bridge
        self.last = None

    def read(self):
        remote = "/data/local/tmp/iris-upgrade-ui.xml"
        self.bridge.shell("uiautomator", "dump", remote)
        data = self.bridge.shell("cat", remote)
        if not data.strip() or len(data) > 2 * 1024 * 1024:
            raise ValueError("Missing or oversized UI hierarchy")
        self.last = ET.fromstring(data)
        return self.last

    def capture(self, name):
        root = self.read()
        ET.ElementTree(root).write(self.bridge.output / (name + ".xml"), encoding="utf-8")
        png = self.bridge.run("exec-out", "screencap", "-p", binary=True)
        if not png.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("Missing actual Android screenshot")
        (self.bridge.output / (name + ".png")).write_bytes(png)
        return root

    def wait(self, predicate, timeout=30):
        deadline = time.monotonic() + timeout
        while True:
            root = self.read()
            if predicate(root):
                return root
            # Decline only the ordinary Android permission prompt in this
            # fresh synthetic account. No account/device setup is automated.
            deny = [n for n in root.iter("node")
                    if n.get("package") in ("com.android.permissioncontroller", "com.google.android.permissioncontroller")
                    and n.get("resource-id", "").endswith(":id/permission_deny_button")]
            if len(deny) == 1:
                self.tap_node(clickable(root, deny))
            if time.monotonic() >= deadline:
                self.capture("failure-ui")
                raise TimeoutError("Expected Android screen did not become ready")
            time.sleep(0.25)

    def tap_node(self, node):
        left, top, right, bottom = bounds(node)
        self.bridge.shell("input", "tap", (left + right) // 2, (top + bottom) // 2)

    def tap(self, **selector):
        root = self.wait(lambda tree: bool(matching(tree, **selector)))
        self.tap_node(clickable(root, matching(root, **selector)))

    def input(self, value, index=0, expected_count=1):
        if not re.fullmatch(r"[A-Za-z0-9 ._-]+", value):
            raise ValueError("Only literal synthetic ASCII fixture input is supported")
        root = self.read()
        fields = matching(root, kind="android.widget.EditText")
        if len(fields) != expected_count or not 0 <= index < len(fields):
            raise ValueError("Unexpected text fields on Android screen")
        self.tap_node(fields[index])
        self.bridge.shell("input", "keycombination", "KEYCODE_CTRL_LEFT", "KEYCODE_A")
        self.bridge.shell("input", "text", value.replace(" ", "%s"))
        self.wait(lambda tree: any(n.get("text") == value for n in
                                  matching(tree, kind="android.widget.EditText")))

    def back(self):
        self.bridge.shell("input", "keyevent", "BACK")

    def open_settings(self):
        root = self.wait(lambda tree: bool(matching(tree, text="Chats"))
                  and bool(matching(tree, description="New chat")))
        header_bottom = bounds(clickable(root, matching(root, description="New chat")))[3]
        avatars = [n for n in matching(root, description="You") if bounds(n)[1] < header_bottom]
        self.tap_node(clickable(root, avatars))
        self.wait(lambda tree: bool(matching(tree, text="My profile")))

    def note_to_self(self):
        self.open_settings()
        self.tap(text="My profile")
        self.tap(text="Note to self")
        self.wait(lambda tree: len(matching(tree, kind="android.widget.EditText")) == 1)

    def chats(self):
        for _ in range(6):
            root = self.read()
            if matching(root, text="Chats") and matching(root, description="New chat"):
                return
            self.back()
        raise RuntimeError("Could not return to chat list")
