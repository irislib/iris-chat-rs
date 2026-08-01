#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="${IRIS_MACOS_GUI_SMOKE_APP:-${ROOT_DIR}/macos/.build/DerivedData/Build/Products/Debug/Iris Chat.app}"
RUN_ID="${IRIS_MACOS_GUI_SMOKE_RUN_ID:-mac-gui-smoke-$(date -u +%Y%m%dT%H%M%SZ)}"
RUN_DIR="${IRIS_MACOS_GUI_SMOKE_RUN_DIR:-${TMPDIR:-/tmp}/iris-macos-gui-smoke-${RUN_ID}}"
SEED_PEER="${IRIS_MACOS_GUI_SMOKE_SEED_PEER:-npub18w35g6gn47qwmryulxzvfucmujvrqqljjpapyl8x0rqaljh6f2usml77dj}"
SKIP_BUILD=0

usage() {
  cat <<'EOF'
usage: scripts/macos_gui_smoke.sh [--skip-build]

Runs a real macOS GUI smoke through Accessibility instead of XCTest. Intended
for the macos-utm VM when Xcode's macOS UI-test runner launches suspended.

Checks:
  - Create-account terms controls are absent on macOS.
  - Moving between messages leaves exactly one hover action dock visible.

Environment:
  IRIS_MACOS_GUI_SMOKE_APP       Built Iris Chat.app path.
  IRIS_MACOS_GUI_SMOKE_RUN_ID    Stable UI-test data run id.
  IRIS_MACOS_GUI_SMOKE_RUN_DIR   Artifact directory.
  IRIS_MACOS_GUI_SMOKE_SEED_PEER Peer user ID used by the seed helper.
  IRIS_MACOS_GUI_SMOKE_KEEP_APP  Set to 1 to leave Iris Chat open.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS GUI smoke requires macOS." >&2
  exit 1
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  "${ROOT_DIR}/scripts/macos-build" macos-build
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "Missing app bundle: $APP_PATH" >&2
  exit 1
fi

mkdir -p "$RUN_DIR"
RESULT_JSON="${RUN_DIR}/result.json"

CONSOLE_USER="$(stat -f %Su /dev/console)"
if [[ -z "$CONSOLE_USER" || "$CONSOLE_USER" == "root" ]]; then
  CONSOLE_USER="$(id -un)"
fi
CONSOLE_UID="$(id -u "$CONSOLE_USER")"

run_gui_user() {
  if sudo -n true >/dev/null 2>&1; then
    sudo -n launchctl asuser "$CONSOLE_UID" sudo -n -u "$CONSOLE_USER" "$@"
  else
    "$@"
  fi
}

set_gui_env() {
  local key="$1"
  local value="$2"
  run_gui_user launchctl setenv "$key" "$value"
}

clear_gui_env() {
  local key="$1"
  run_gui_user launchctl unsetenv "$key" >/dev/null 2>&1 || true
}

cleanup() {
  if [[ "${IRIS_MACOS_GUI_SMOKE_KEEP_APP:-0}" != "1" ]]; then
    pkill -x "Iris Chat" 2>/dev/null || true
  fi
  clear_gui_env IRIS_UI_TEST_RESET
  clear_gui_env IRIS_UI_TEST_RUN_ID
  clear_gui_env IRIS_UI_TEST_BYPASS_KEYCHAIN
  clear_gui_env IRIS_DISABLE_NOTIFICATIONS_FOR_AUTOMATION
  clear_gui_env IRIS_UI_TEST_SEED_PEER
  clear_gui_env IRIS_UI_TEST_SEED_COUNT
}
trap cleanup EXIT

pkill -x "Iris Chat" 2>/dev/null || true

set_gui_env IRIS_UI_TEST_RESET 1
set_gui_env IRIS_UI_TEST_RUN_ID "$RUN_ID"
set_gui_env IRIS_UI_TEST_BYPASS_KEYCHAIN 1
set_gui_env IRIS_DISABLE_NOTIFICATIONS_FOR_AUTOMATION 1
set_gui_env IRIS_UI_TEST_SEED_PEER "$SEED_PEER"
set_gui_env IRIS_UI_TEST_SEED_COUNT 2

run_gui_user open -n -a "$APP_PATH"

message_coords="$(
  osascript -l JavaScript <<'JXA'
const se = Application('System Events');

function proc() {
  return se.processes.byName('Iris Chat');
}

function attr(el, name) {
  try {
    const value = el.attributes.byName(name).value();
    return value === null || value === undefined ? '' : String(value);
  } catch (error) {
    return '';
  }
}

function rawAttr(el, name) {
  try {
    return el.attributes.byName(name).value();
  } catch (error) {
    return null;
  }
}

function children(el) {
  try {
    return el.uiElements();
  } catch (error) {
    return [];
  }
}

function find(el, predicate) {
  if (predicate(el)) return el;
  for (const child of children(el)) {
    const found = find(child, predicate);
    if (found) return found;
  }
  return null;
}

function findAll(el, predicate, out = []) {
  if (predicate(el)) out.push(el);
  for (const child of children(el)) findAll(child, predicate, out);
  return out;
}

function roots() {
  try {
    return proc().windows();
  } catch (error) {
    return [];
  }
}

function waitFor(predicate, timeoutSeconds) {
  const deadline = Date.now() + timeoutSeconds * 1000;
  while (Date.now() < deadline) {
    try {
      proc().frontmost = true;
    } catch (error) {}
    for (const root of roots()) {
      const found = find(root, predicate);
      if (found) return found;
    }
    delay(0.25);
  }
  return null;
}

function waitId(identifier, timeoutSeconds) {
  return waitFor((el) => attr(el, 'AXIdentifier') === identifier, timeoutSeconds);
}

function existsId(identifier) {
  return waitId(identifier, 0.2) !== null;
}

function press(el) {
  try {
    el.actions.byName('AXPress').perform();
  } catch (error) {
    el.click();
  }
}

function frame(el) {
  const position = rawAttr(el, 'AXPosition');
  const size = rawAttr(el, 'AXSize');
  return {
    x: position ? position[0] : null,
    y: position ? position[1] : null,
    w: size ? size[0] : null,
    h: size ? size[1] : null,
  };
}

function visibleIds() {
  const ids = [];
  for (const root of roots()) {
    findAll(root, (el) => attr(el, 'AXIdentifier') !== '', ids);
  }
  return ids
    .map((el) => `${attr(el, 'AXRole')}:${attr(el, 'AXIdentifier')}:${attr(el, 'AXValue') || attr(el, 'AXTitle')}`)
    .slice(0, 100)
    .join('\n');
}

const create = waitId('welcomeCreateAction', 30);
if (!create) throw new Error(`welcomeCreateAction missing:\n${visibleIds()}`);
press(create);

if (!waitId('createAccountScreen', 15)) {
  throw new Error(`createAccountScreen missing:\n${visibleIds()}`);
}
if (existsId('onboardingTermsAgreementToggle') || existsId('onboardingTermsNotice')) {
  throw new Error('macOS create-account screen displayed iOS-only terms controls');
}

const nameField = waitId('signupNameField', 15);
if (!nameField) throw new Error(`signupNameField missing:\n${visibleIds()}`);
nameField.value = 'Mac GUI Smoke';

const generate = waitId('generateKeyButton', 15);
if (!generate) throw new Error(`generateKeyButton missing:\n${visibleIds()}`);
const enabledDeadline = Date.now() + 10000;
while (attr(generate, 'AXEnabled') !== 'true' && Date.now() < enabledDeadline) {
  delay(0.25);
}
if (attr(generate, 'AXEnabled') !== 'true') {
  throw new Error(`generateKeyButton never enabled; AXEnabled=${attr(generate, 'AXEnabled')}`);
}
press(generate);

const ready = waitFor((el) => {
  const id = attr(el, 'AXIdentifier');
  return id === 'chatListNewChatButton' || id === 'desktopNewChatRow' || id === 'chatMessageInput';
}, 80);
if (!ready) throw new Error(`chat list or chat input never appeared:\n${visibleIds()}`);

if (attr(ready, 'AXIdentifier') !== 'chatMessageInput') {
  const row = waitFor((el) => attr(el, 'AXIdentifier').startsWith('chatRow-'), 45);
  if (!row) throw new Error(`seeded chat row missing:\n${visibleIds()}`);
  press(row);
}

if (!waitId('chatMessageInput', 30)) {
  throw new Error(`chatMessageInput missing after opening seeded chat:\n${visibleIds()}`);
}

let messages = [];
const messagesDeadline = Date.now() + 30000;
while (messages.length < 2 && Date.now() < messagesDeadline) {
  const byId = {};
  for (const root of roots()) {
    const matches = findAll(root, (el) => attr(el, 'AXIdentifier').startsWith('chatMessage-'));
    for (const match of matches) {
      const id = attr(match, 'AXIdentifier');
      const candidateFrame = frame(match);
      const candidateArea = (candidateFrame.w || 0) * (candidateFrame.h || 0);
      const existingFrame = byId[id] ? frame(byId[id]) : null;
      const existingArea = existingFrame ? (existingFrame.w || 0) * (existingFrame.h || 0) : -1;
      if (candidateArea > existingArea) byId[id] = match;
    }
  }
  messages = Object.values(byId).sort((lhs, rhs) => frame(lhs).y - frame(rhs).y);
  if (messages.length < 2) delay(0.25);
}
if (messages.length < 2) throw new Error(`seed messages missing:\n${visibleIds()}`);

const firstMessage = messages[0];
const lastMessage = messages[messages.length - 1];

const firstFrame = frame(firstMessage);
const lastFrame = frame(lastMessage);
[
  Math.round(firstFrame.x + firstFrame.w / 2),
  Math.round(firstFrame.y + firstFrame.h / 2),
  Math.round(lastFrame.x + lastFrame.w / 2),
  Math.round(lastFrame.y + lastFrame.h / 2),
  attr(lastMessage, 'AXIdentifier'),
].join(' ');
JXA
)"

first_mouse_x="$(printf '%s' "$message_coords" | awk '{print $1}')"
first_mouse_y="$(printf '%s' "$message_coords" | awk '{print $2}')"
last_mouse_x="$(printf '%s' "$message_coords" | awk '{print $3}')"
last_mouse_y="$(printf '%s' "$message_coords" | awk '{print $4}')"
last_message_id="$(printf '%s' "$message_coords" | awk '{print $5}')"
if [[ -z "$first_mouse_x" || -z "$first_mouse_y" || -z "$last_mouse_x" || -z "$last_mouse_y" || -z "$last_message_id" ]]; then
  echo "Unable to resolve message hover coordinates: $message_coords" >&2
  exit 1
fi

run_gui_user /usr/bin/swift - "$first_mouse_x" "$first_mouse_y" "$last_mouse_x" "$last_mouse_y" <<'SWIFT'
import CoreGraphics
import Foundation

let source = CGEventSource(stateID: .hidSystemState)
for index in stride(from: 1, through: 3, by: 2) {
    let x = Double(CommandLine.arguments[index])!
    let y = Double(CommandLine.arguments[index + 1])!
    let event = CGEvent(
        mouseEventSource: source,
        mouseType: .mouseMoved,
        mouseCursorPosition: CGPoint(x: x, y: y),
        mouseButton: .left
    )
    event?.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.2)
}
SWIFT

verification_json="$(
  IRIS_MACOS_GUI_SMOKE_RESULT_RUN_ID="$RUN_ID" \
  IRIS_MACOS_GUI_SMOKE_LAST_MESSAGE_ID="$last_message_id" \
  osascript -l JavaScript <<'JXA'
ObjC.import('stdlib');
const se = Application('System Events');
const proc = se.processes.byName('Iris Chat');
proc.frontmost = true;
const runId = ObjC.unwrap($.getenv('IRIS_MACOS_GUI_SMOKE_RESULT_RUN_ID')) || '';
const lastMessageId = ObjC.unwrap($.getenv('IRIS_MACOS_GUI_SMOKE_LAST_MESSAGE_ID')) || '';

function attr(el, name) {
  try {
    const value = el.attributes.byName(name).value();
    return value === null || value === undefined ? '' : String(value);
  } catch (error) {
    return '';
  }
}

function rawAttr(el, name) {
  try {
    return el.attributes.byName(name).value();
  } catch (error) {
    return null;
  }
}

function children(el) {
  try {
    return el.uiElements();
  } catch (error) {
    return [];
  }
}

function findAll(el, predicate, out = []) {
  if (predicate(el)) out.push(el);
  for (const child of children(el)) findAll(child, predicate, out);
  return out;
}

function frame(el) {
  const position = rawAttr(el, 'AXPosition');
  const size = rawAttr(el, 'AXSize');
  return {
    x: position ? position[0] : null,
    y: position ? position[1] : null,
    w: size ? size[0] : null,
    h: size ? size[1] : null,
  };
}

const allElements = [];
for (const root of proc.windows()) findAll(root, () => true, allElements);
const message = allElements
  .filter((el) => attr(el, 'AXIdentifier') === lastMessageId)
  .sort((lhs, rhs) => {
    const lhsFrame = frame(lhs);
    const rhsFrame = frame(rhs);
    return (rhsFrame.w || 0) * (rhsFrame.h || 0) - (lhsFrame.w || 0) * (lhsFrame.h || 0);
  })[0];
const moreButtons = allElements.filter((el) => attr(el, 'AXIdentifier') === 'messageMoreButton');
const infoButtons = allElements.filter((el) => attr(el, 'AXIdentifier') === 'messageInfoButton');
const replyButtons = allElements.filter((el) => attr(el, 'AXIdentifier') === 'messageReplyButton');
const reactButtons = allElements.filter((el) => attr(el, 'AXIdentifier') === 'messageReactButton');
const more = moreButtons[0];
const info = infoButtons[0];
const reply = replyButtons[0];
const react = reactButtons[0];

if (!message) throw new Error('seed message disappeared before hover verification');
if (!more) throw new Error('messageMoreButton did not appear after mouse hover');
if (!info) throw new Error('messageInfoButton did not appear after mouse hover');
if (!reply) throw new Error('messageReplyButton did not appear after mouse hover');
if (!react) throw new Error('messageReactButton did not appear after mouse hover');
if (
  moreButtons.length !== 1 ||
  infoButtons.length !== 1 ||
  replyButtons.length !== 1 ||
  reactButtons.length !== 1
) {
  throw new Error(
    `expected one message action dock after moving between messages; ` +
    `react=${reactButtons.length} reply=${replyButtons.length} ` +
    `info=${infoButtons.length} more=${moreButtons.length}`
  );
}

const messageFrame = frame(message);
const moreFrame = frame(more);
const infoFrame = frame(info);
const replyFrame = frame(reply);
const reactFrame = frame(react);
const actionGap = messageFrame.x - (moreFrame.x + moreFrame.w);

// SwiftUI may expose the message identifier on the trailing delivery glyph
// instead of the bubble container. Account for the 480pt desktop bubble cap
// while still requiring the action dock to be on the message's left side.
if (!(actionGap > 0 && actionGap < 540)) {
  throw new Error(`outgoing action dock is not beside the bubble; actionGap=${actionGap}`);
}
if (!(
  reactFrame.x < replyFrame.x &&
  replyFrame.x < infoFrame.x &&
  infoFrame.x < moreFrame.x &&
  moreFrame.x < messageFrame.x
)) {
  throw new Error('outgoing action dock buttons are not laid out to the left of the message');
}

JSON.stringify({
  status: 'passed',
  runId,
  termsToggleVisible: false,
  termsNoticeVisible: false,
  messageFrame,
  reactFrame,
  replyFrame,
  infoFrame,
  moreFrame,
  actionGap,
}, null, 2);
JXA
)"

printf '%s\n' "$verification_json" >"$RESULT_JSON"
printf '%s\n' "$verification_json"
echo "macOS GUI smoke passed"
echo "run_dir=${RUN_DIR}"
