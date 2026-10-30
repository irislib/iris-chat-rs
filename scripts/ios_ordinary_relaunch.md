# Ordinary iOS replacement launch evidence

These Python standard-library helpers qualify a single ordinary CoreDevice
foreground launch with `--terminate-existing`. They do not install, sign,
operate a phone on import, or grant release acceptance. The unchanged tagged
`ios_runtime_acceptance.py` and `RELEASE.md` remain authoritative.

Run offline qualification with `qualify_ios_ordinary_relaunch.py --out <new-private-directory>
--tagged-validator <exact-tag-validator>`. The directory must not exist. The
runner retains test output, exit status, source hashes and the unchanged tagged
validator binding. It runs synthetic tests and mocked promotion checks only.
Optional `--provenance-qualification <private-record>` binds an already reviewed
installation/provenance qualification; omission leaves that dependency unbound.
Neither form establishes any physical runtime result.

## Private inputs

Keep all actual targets, signing records, logs, screenshots, accounts and receipts
outside Git. `validate_distribution(binding, candidate)` consumes a normalized
`iris-ordinary-relaunch-input-v1` record from separately qualified production
AdHoc provisioning/install checks. Its strict structure is exercised by the
synthetic `DistributionFixture` in `test_ios_ordinary_relaunch.py`:

- Exact candidate, physical device identifier, CoreDevice result identifier,
  bundle, installed URL, previous build and installation time. The two device
  identifiers may differ; their verified mapping belongs to the private input.
- Every existing v3 AdHoc policy and provenance digest, with bindings to the
  authorization, release acceptance, original IPA, manifest, preparation,
  installation, original signing and profiles.
- Matching original/re-signed UUID, load-command and unsigned-content records
  for the app and both extensions, with the unchanged v3 digest encoding.
- A separately qualified provenance record and release-owner review binding the
  exact candidate, target, inputs, code records and checker sources.

This helper verifies those bindings and checks consistency. It does **not**
replace the qualified code-signing/profile/install checks with matching digest
strings. Missing source receipts or an unreviewed provenance record remain a
blocker; never hand-author passing evidence to fill that gap.

## One reserved attempt

`OrdinaryForegroundRelaunch.run_once` accepts small capture/transport callbacks.
`ios_relaunch_adapters.py` supplies the real bounded CoreDevice, syslog and USB
Home adapters. The release owner must hash-bind their actual tools and private
admission, reserve the physical device and preserve the first failure.
Callbacks must provide real observations rather than inferred fields:

1. `prearm()` returns the acknowledged nonce, exact device/bundle/build and
   `armed: true`, without launching/activating the app or creating an XCTest
   launch assertion. `snapshot()` returns the target process inventory, actual
   foreground PID and capture time.
2. `capture_prefixes()` returns ready, complete startup/lifecycle log snapshots.
   The controller rechecks foreground freshness immediately before launch.
3. `launch(command)` issues exactly the supplied ordinary CoreDevice request,
   without console/debugger/environment/argument overrides, and returns its raw
   result. There is no retry, including after an uncertain outcome.
4. After an in-memory check of the returned target, ordinary options and fresh
   PID, `trigger(armed, proof)` sends one prearmed Home or
   Lock action. Its result must contain the same nonce/build/new PID,
   `sent_count: 1` and `acknowledged: true`. This acknowledgement proves no
   background entry, overlap or suspension by itself. Actual process inventory
   and full launch validation follow the trigger. Failure there invalidates the
   attempt even though Home was already sent; no second attempt is permitted.

`OwnedWindow` requires `iris-relaunch-adapters-v1` admission with the exact
candidate/distribution binding, target, direct-parent owner, one new output root, current deadline,
host disk reserve, output cap (at most 100 MiB) and tools. Both native-lab leases
must belong to that live direct parent. Tool bindings name the direct CoreDevice
executable, idevicesyslog, isolated Python interpreter, retained disabled Home
helper, its existing qualification and a separately reviewed enabled copy.
The adapter verifies the retained helper/qualification pins and permits only
the single execution-enable line change; it never creates that copy itself.
The helper keeps its qualified USB-only transport and pinned dependency checks.
There is no new UI runner, pairing, package installation or control fallback.

Create `CoreDevice(window)`, `Capture(window, core)` and `PrearmedHome(window)`.
Start capture before arming Home. Wire `snapshot=capture.snapshot`,
`prearm=home.arm`, `capture_prefixes=capture.prefixes`, `launch=core.run` and
`trigger=home.fire`. The two syslog streams must explicitly report connection
to the exact phone, and the existing process needs actual ordered SpringBoard
foreground evidence. An idle process with no such evidence does not qualify.
The returned Home acknowledgement represents the helper's local writes and
clean transport closure, **not** an OS acknowledgement or observed device effect.

Use the validated physical-device identifier for log connections and the bound
CoreDevice identifier for returned launch results. Do not infer foreground state
from a successful launch or a process being present. Current process inventory
supplies PID/executable, not an audit token; do not invent per-poll audit tokens.

## Final evidence

Capture append-only raw-log prefixes and PID/executable samples no more than
8 seconds apart. Preserve complete streams, readiness/disconnection/error
records, live-logger observations and empty stderr. Unsupported log formats
fail; extend and qualify a parser explicitly rather than silently dropping them.

`evaluate_runtime(distribution_binding, candidate, attempt, resume, capture)`
revalidates provenance, actual launch results, fresh process, real native overlap,
135 seconds continuously suspended and 15 seconds resumed. It verifies the real
ordinary resume launch's audit token against the initial launch and binds resume
placement to the captured host monotonic interval. Paired host UTC/monotonic
clocks must agree within 250ms; device log timestamps are never compared with
host clocks. Clock jumps fail instead of extending a freshness/coverage bound.
After the observation, close the owned window and obtain
`capture.final_capture(samples)`. This retains the untrimmed final log tails and
a hash-bound candidate/target process-group closure receipt; a live group or
cleanup failure invalidates final evaluation. A reaped process leader's numeric
group ID is never used to signal a potentially unrelated reused process.

`compose_adhoc_proof` consumes a hash-bound
`iris-ordinary-relaunch-capture-v1` record containing `attempt`, `resume` and
`capture`, plus the fresh helper qualification. It replays the evidence and
produces only the AdHoc portion of v3. Every other lifecycle, preserved-state,
crash, CPU, startup and sending check is still required for the exact candidate.
Use the tagged workflow only after the release owner accepts the complete
private evidence. Existing failed attempts and historic qualifications remain
unchanged; these helpers make no claim to reproduce their missing source bytes.
