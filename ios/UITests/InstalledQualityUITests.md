# Installed-app quality observations

`InstalledQualityUITests.swift` is a separate physical-iPhone UI runner for the
startup and sending evidence required by `RELEASE.md`. It does not establish
installed-build provenance, CPU quality, lifecycle survival or release approval.
Use only after the release owner binds the installed app to the exact tagged IPA
and manifest. The selectors and own-profile action were reviewed against
`v2026.10.30`; the runner is separate tooling, not a replacement app build.

Build this one Swift file in a standalone UI-test bundle with no Iris app target,
app dependency, `TEST_HOST`, `BUNDLE_LOADER` or target-app installation. Do not use
the repository's normal app/UI-test scheme. Inspect the generated `.xctestrun`
and installation plan before execution; only the separate runner may be
installed. Never launch, terminate, activate or inject arguments/environment into
Iris through XCTest. No reset, reinstall, debugger, Keychain or profile changes.
Run only one selected test, with retries and parallel testing disabled. Keep
these UI tests outside every CPU and background-suspension measurement window.
The release owner controls device access and ordinary external launch.

Pass these values to the **runner only**, through a private, access-restricted
`.xctestrun` environment (never put message bodies or drafts in command logs):

- `IRIS_INSTALLED_QUALITY_MODE`: `startup`, `self-send`, `pending-self-send`, `return-original` or `discover-row`.
- `IRIS_QUALITY_RUN_ID`: fresh UUID; retain it across the launch/observation receipts.
- `IRIS_QUALITY_CANDIDATE_BINDING_SHA256`: digest of the independently verified
  private installed-candidate receipt, binding the device, exact source, IPA,
  manifest and installation. Supplying this digest does not verify that receipt.
- `IRIS_QUALITY_ORIGINAL_CHAT_ROW`: exact existing `chatRow-` plus 12-character ID;
  required for startup/self-send and unused by discovery.
- `IRIS_ACCEPTANCE_MESSAGE_IDENTIFIER`, `IRIS_ACCEPTANCE_MESSAGE_BODY`,
  `IRIS_ACCEPTANCE_DRAFT`: exact existing visible message and composer value in
  that original chat, with a nonempty original draft for these checks. Obtain
  these from prior authorized private inspection; do not seed or alter state.

Original-state preservation compares the exact composer string. An empty
original draft may appear as a nil UIKit value and is not qualified by this
runner. The separate empty Note to self composer check below handles nil values
and verifies the visible controls before sending.

For `testDiscoverOriginalChatRowAndReturn`, start in the exact original chat with
its existing message and nonempty draft visible. The draft must be unchanged by
whitespace trimming. The runner reads the actual plain header title, goes Back
once and matches exactly one fully visible UIKit row by title and the exact
`Draft: <draft>` label component, allowing the tagged optional `Nearby` component.
It records the real row identifier/label privately, opens only that unique row,
and verifies the original message/draft again. No typing, sending, searching or
scrolling occurs. Ambiguous headers, comma-space inside title/draft, missing draft
previews or multiple matches fail without a guessed return target. On success,
read `return-row-discovered` from the private observation attachment and bind its
identifier/evidence into the later startup/self-send runner configuration.

For `testObserveExternalColdStartup`, stop Iris externally before starting the
runner. Wait for `IRIS_INSTALLED_QUALITY_READY=<run UUID>`. Only then launch the verified installed app ordinarily from the external driver,
without a debugger, launch assertion or overrides. Retain external launch/PID
and crash evidence alongside the observations. The runner starts with Iris not
running, takes screenshots before each accessibility query, and observes until
the exact message/draft stays visible for two seconds. If startup returns to the
chat list, it opens only the exact supplied row once. It fails observed login
controls, process disappearance, or failure to reach preserved state within its
60-second/120-sample capture budget; it does not retry or repair a failure.

Screenshots and accessibility queries have separate monotonic bounds plus wall
clock anchors. XCTest may wait for quiescence: the requested 0.1-second pause is
not a promised sampling interval, and the runner cannot rule out a brief flash
between captures. The owner reviews the actual timed images/state observations
and their gaps, binds that review and the focused login/loading regression to
the exact candidate, and rejects any observed login flash. Record only what was
observed; do not claim proof about unsampled instants. An existing supported
video capture may supplement the evidence but is not a new release gate. A
runner pass supplies observations, not the release owner's acceptance decision.

For `testSendOneNoteToSelfAndReturn`, Iris must already be foreground in the
specified original chat, with the expected message and draft visible. The runner
first returns through the exact chat row and checks that message/draft, then uses
Settings → Profile → the dedicated `myProfileNoteToSelfButton`. In the tagged app
that action opens the account's own public-key chat; a contact merely named
“Note to self” is never selected. The direct own-profile route and closed Settings
must be verified before typing and sending. The runner rejects a nonempty self draft,
attachments, edits, replies, voice recording, pending send, alerts or sheets.
The original chat must also have no pending attachment/edit/reply/voice state,
and its exact return row must be uniquely visible without searching or scrolling.
If Note to self is the original chat and contains a draft, the check intentionally
stops: do not clear or temporarily replace the draft to make it run.

The only text typed is `Iris quality check <run UUID>`. It taps Send once, records
tap-start/tap-return and first visible exact-message timestamps with screenshots,
then returns to the original row and checks the original message/draft. The
reported send-to-visible duration is an **observed upper bound**, including UI
automation/capture delays, not a transport latency or invented pass threshold.
The 20-second wait is an execution budget. The owner reviews the focused flow
and actual timing for the candidate-bound quality receipt.

On any failure, preserve the first xcresult, final screenshot, event attachment,
external logs and any supplementary recording. The runner performs no cleanup, deletion, resend,
automatic navigation recovery or relaunch; a partially typed synthetic draft may
remain in Note to self. Diagnose the retained state before any new attempt. On
success the original chat/draft is rechecked and the single synthetic self-message
remains. Keep all evidence private: captures and event nodes contain chat text.
Hash the actual runner source, `.xctestrun`, xcresult, external observations and
review into the release evidence; do not publish them or substitute a success
flag for review. No native build or device execution is part of this source change.

The self-send observer establishes its recipient by the tagged own-profile
`myProfileNoteToSelfButton` action, which opens the account's own public key.
Display titles and call controls can load asynchronously and do not authenticate
the recipient. Do not use them as fixed readiness predicates.

`return-original` selects only `testReturnToOriginalConversation`: capture the
current composer, navigate Back once, open the previously observed exact
row, then verify the original message and draft. It never types or sends and does not require an empty draft to navigate Back.

Before typing or accepting a cleared sent draft, match the tagged composer:
its value is nil/empty or matches the displayed placeholder, the Message placeholder overlaps the text view, Send
is absent, and the voice-record action is present. A nonempty draft, including
text equal to the placeholder, must not pass. Existing TestFlight draft tests
also treat a nil UIKit value as empty; the visible controls corroborate it here.

`pending-self-send` selects `testFinishPendingNoteToSelfAndReturn` only after the
release owner reviews a failed attempt that typed but did not send. Bind
`IRIS_QUALITY_PENDING_RUN_ID` to that exact failed run; use a fresh observation
`IRIS_QUALITY_RUN_ID`. The helper captures and checks the exact pending synthetic
draft, verifies the original conversation, reopens the direct own-profile action,
and checks the unchanged draft and absence of an exact matching message. It types
nothing and never clears or replaces a draft. It sends once, records the same
visibility bounds, and verifies the original conversation again. Any failure
preserves evidence and state; there is no automatic retry.
