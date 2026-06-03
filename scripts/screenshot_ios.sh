#!/usr/bin/env bash
#
# Capture App Store screenshots on the iOS simulator.
#
# Drives the ScreenshotTests XCUITest with the IRIS_UI_TEST_SCREENSHOT_FIXTURE
# state-override turned on, then unpacks named XCTAttachments from the
# generated .xcresult bundle into dist/ios-screenshots/<device>/.
#
# Usage:
#   ./scripts/screenshot_ios.sh                # default: iPhone + iPad
#   ./scripts/screenshot_ios.sh "iPhone 17 Pro Max"
#   ./scripts/screenshot_ios.sh --list         # show available simulators
#
# Apple requires 6.9" iPhone + 13" iPad screenshots; the defaults satisfy that.

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IOS_DIR="$ROOT/ios"
PROJECT="$IOS_DIR/IrisChat.xcodeproj"
SCHEME="IrisChat"
TEST_TARGET="IrisChatUITests/ScreenshotTests/testCaptureAppStoreScreenshots"
OUT_DIR="$ROOT/dist/ios-screenshots"
DERIVED_DATA="$IOS_DIR/.build/screenshot-derived-data"

DEFAULT_DEVICES=(
  "iPhone 17 Pro Max"
  "iPad Pro 13-inch (M5)"
)

usage() {
  cat <<'EOF'
Usage: scripts/screenshot_ios.sh [options] [simulator-name ...]

Options:
  --list           List available iOS simulators and exit
  --keep           Keep the .xcresult bundles (default: delete after extract)
  --derived-data   Override derived data path
  --appearance     'light' (default), 'dark', or 'both' to capture twice
                   ('both' writes light to <slug>/ and dark to <slug>-dark/)

If no simulator names are given, runs on:
  iPhone 17 Pro Max
  iPad Pro 13-inch (M5)
EOF
}

KEEP_XCRESULT=0
APPEARANCE="light"
DEVICES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      xcrun simctl list devices available | grep -E "^\s+(iPhone|iPad)" || true
      exit 0
      ;;
    --keep)
      KEEP_XCRESULT=1
      shift
      ;;
    --derived-data)
      DERIVED_DATA="$2"
      shift 2
      ;;
    --appearance)
      APPEARANCE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      DEVICES+=("$1")
      shift
      ;;
  esac
done

if [[ ${#DEVICES[@]} -eq 0 ]]; then
  DEVICES=("${DEFAULT_DEVICES[@]}")
fi

if ! command -v xcrun >/dev/null; then
  echo "xcrun not found. Install Xcode command line tools." >&2
  exit 1
fi

resolve_udid() {
  local name="$1"
  xcrun simctl list -j devices available |
    /usr/bin/python3 -c "
import json, sys
name = sys.argv[1]
data = json.load(sys.stdin)
for runtime, devices in sorted(data.get('devices', {}).items(), reverse=True):
    for d in devices:
        if d.get('name') == name and d.get('isAvailable', True):
            print(d.get('udid'))
            sys.exit(0)
sys.exit(1)
" "$name"
}

shutdown_stale_ios_simulators() {
  if [[ "${IRIS_E2E_CLOSE_STALE_IOS_SIMS:-1}" == "0" ]]; then
    return 0
  fi

  local -a keep=("$@")
  local udid=""
  while IFS= read -r udid; do
    local keep_udid=""
    local should_keep=0
    for keep_udid in "${keep[@]}"; do
      if [[ "${udid}" == "${keep_udid}" ]]; then
        should_keep=1
        break
      fi
    done
    if [[ "${should_keep}" -eq 0 ]]; then
      if pgrep -fl xcodebuild 2>/dev/null | grep -F "id=${udid}" >/dev/null 2>&1; then
        echo "▶︎  Keeping active simulator $udid"
        continue
      fi
      echo "▶︎  Shutting down stale simulator $udid"
      xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
    fi
  done < <(xcrun simctl list devices booted | sed -n 's/.*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p')
}

shutdown_owned_ios_simulators() {
  if [[ "${IRIS_E2E_KEEP_IOS_SIMS:-0}" == "1" ]]; then
    return 0
  fi

  local udid=""
  for udid in "$@"; do
    [[ -n "$udid" ]] || continue
    xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  done
}

slugify() {
  printf '%s' "$1" |
    /usr/bin/python3 -c "
import re, sys
s = sys.stdin.read().strip().lower()
s = re.sub(r'[^a-z0-9]+', '-', s).strip('-')
print(s)
"
}

extract_screenshots() {
  local xcresult="$1"
  local out_dir="$2"
  mkdir -p "$out_dir"
  /usr/bin/python3 - "$xcresult" "$out_dir" <<'PY'
import json
import os
import subprocess
import sys

xcresult, out_dir = sys.argv[1:3]

# Xcode 16+ defaults `xcresulttool get` to a new subcommand layout that
# refuses the JSON walk we use here. The deprecated legacy mode still
# works and emits the schema this script targets.
LEGACY = ["--legacy"]

def get(ref_id=None):
    cmd = ["xcrun", "xcresulttool", "get", *LEGACY, "--format", "json", "--path", xcresult]
    if ref_id is not None:
        cmd += ["--id", ref_id]
    return json.loads(subprocess.run(cmd, check=True, stdout=subprocess.PIPE).stdout)

def export(ref_id, dest):
    subprocess.run(
        [
            "xcrun", "xcresulttool", "export",
            *LEGACY,
            "--type", "file",
            "--path", xcresult,
            "--id", ref_id,
            "--output-path", dest,
        ],
        check=True,
    )

def values(node, key):
    return node.get(key, {}).get("_values", []) if isinstance(node, dict) else []

def value(node, *keys):
    cur = node
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key, {})
    if isinstance(cur, dict):
        return cur.get("_value")
    return cur

found = 0

def walk_activity(activity):
    global found
    for attachment in values(activity, "attachments"):
        name = value(attachment, "name") or ""
        if not (name.startswith("screenshot-") or name.startswith("debug-")):
            continue
        ref = value(attachment, "payloadRef", "id")
        if not ref:
            continue
        slug = name[len("screenshot-"):] if name.startswith("screenshot-") else name
        dest = os.path.join(out_dir, f"{slug}.png")
        export(ref, dest)
        print(f"  → {dest}")
        found += 1
    for sub in values(activity, "subactivities"):
        walk_activity(sub)

def walk_test(node):
    type_name = value(node, "_type", "_name") or ""
    summary_ref = value(node, "summaryRef", "id")
    if summary_ref:
        summary = get(summary_ref)
        for activity in values(summary, "activitySummaries"):
            walk_activity(activity)
    for sub in values(node, "subtests"):
        walk_test(sub)
    for sub in values(node, "subtestGroups"):
        walk_test(sub)
    for sub in values(node, "tests"):
        walk_test(sub)

root = get()
for action in values(root, "actions"):
    tests_ref = value(action, "actionResult", "testsRef", "id")
    if not tests_ref:
        continue
    tests = get(tests_ref)
    for summary in values(tests, "summaries"):
        for testable in values(summary, "testableSummaries"):
            for test in values(testable, "tests"):
                walk_test(test)

if found == 0:
    print("warning: no screenshot- attachments found in xcresult", file=sys.stderr)
    sys.exit(2)
PY
}

# IRIS_APP_VERSION_CODE / IRIS_XCODE_MARKETING_VERSION are normally set
# by release.env. Provide harmless placeholders so the simulator install
# doesn't reject the extension for an empty CFBundleVersion.
export IRIS_APP_VERSION_CODE="${IRIS_APP_VERSION_CODE:-0}"
export IRIS_XCODE_MARKETING_VERSION="${IRIS_XCODE_MARKETING_VERSION:-0.0.0}"

# Build-for-testing once using the first device's UDID. The bundled
# xcframework only ships arm64 slices, so `generic/platform=iOS Simulator`
# (which also wants x86_64) won't link.
mkdir -p "$DERIVED_DATA" "$OUT_DIR"
first_udid=""
TARGET_UDIDS=()
for d in "${DEVICES[@]}"; do
  candidate="$(resolve_udid "$d" || true)"
  if [[ -n "${candidate:-}" ]]; then
    TARGET_UDIDS+=("$candidate")
    if [[ -z "$first_udid" ]]; then
      first_udid="$candidate"
    fi
  fi
done
if [[ -z "$first_udid" ]]; then
  echo "No matching simulator booted; nothing to do." >&2
  exit 1
fi
shutdown_stale_ios_simulators "${TARGET_UDIDS[@]}"
cleanup() {
  local exit_code=$?
  shutdown_owned_ios_simulators "${TARGET_UDIDS[@]}"
  exit "${exit_code}"
}
trap cleanup EXIT

echo "▶︎  Building tests against $first_udid …"
xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -destination "id=$first_udid" \
  -derivedDataPath "$DERIVED_DATA" \
  -only-testing "$TEST_TARGET" \
  ONLY_ACTIVE_ARCH=YES \
  build-for-testing \
  -quiet

case "$APPEARANCE" in
  light|dark) APPEARANCES=("$APPEARANCE") ;;
  both) APPEARANCES=("light" "dark") ;;
  *)
    echo "Unknown --appearance value: $APPEARANCE (expected light|dark|both)" >&2
    exit 1
    ;;
esac

for device in "${DEVICES[@]}"; do
  udid="$(resolve_udid "$device" || true)"
  if [[ -z "${udid:-}" ]]; then
    echo "⚠︎  Simulator not found: $device — run with --list to see options" >&2
    continue
  fi
  base_slug="$(slugify "$device")"
  echo "▶︎  Booting $device ($udid)"
  xcrun simctl boot "$udid" >/dev/null 2>&1 || true

for appearance in "${APPEARANCES[@]}"; do
  slug="$base_slug"
  [[ "$appearance" == "dark" ]] && slug="${base_slug}-dark"
  device_out="$OUT_DIR/$slug"
  rm -rf "$device_out"
  mkdir -p "$device_out"
  xcresult="$device_out/run.xcresult"
  rm -rf "$xcresult"

  echo "▶︎  Setting $device appearance to $appearance"
  xcrun simctl ui "$udid" appearance "$appearance" >/dev/null 2>&1 || true

  echo "▶︎  Running ScreenshotTests on $device ($appearance)"
  # Test failures are non-fatal — the run still produces an .xcresult,
  # and we want to extract whichever screenshots did make it. The final
  # exit code from this script reflects whether anything was extracted.
  set +e
  xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -destination "id=$udid" \
    -derivedDataPath "$DERIVED_DATA" \
    -only-testing "$TEST_TARGET" \
    -resultBundlePath "$xcresult" \
    test-without-building \
    -quiet
  xcodebuild_status=$?
  set -e
  if [[ $xcodebuild_status -ne 0 ]]; then
    echo "⚠︎  xcodebuild exited $xcodebuild_status — extracting whatever shipped"
  fi

  echo "▶︎  Extracting screenshots → $device_out"
  extract_screenshots "$xcresult" "$device_out" || true

  if [[ $KEEP_XCRESULT -eq 0 ]]; then
    rm -rf "$xcresult"
  fi
done
done

echo "✓  Done. Screenshots are under $OUT_DIR"
