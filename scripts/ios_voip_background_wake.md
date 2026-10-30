# Cold background wake evidence

`ios_voip_background_wake.py` prepares one private physical-device observation
for `cold_background_launch` in the unchanged tagged v3 release contract. It
does not promote a release, create a public acceptance receipt, or replace the
separate early-restoration-overlap, crash, CPU, upgrade and preserved-state gates.
Importing it and running its qualification do not contact a device or APNs.

The route uses the production PushKit unavailable-call behavior: the payload has
only `aps.content-available` and a fresh opaque request ID. It contains no event,
message, call invitation or participant identity. The current app reports and
ends an unavailable call through CallKit. This can cause transient system call
UI or sound; the payload is not claimed to be silent. It does not send a message
or modify a draft. Existing message, draft, account and call-history state still
need a separate visual comparison after the measured process resumes.

The sender uses Node's built-in HTTP/2 and crypto modules. It opens one verified
TLS connection to production APNs, makes one POST with the VoIP topic, priority
10 and expiration 0, and never retries. Tokens, credential metadata and the
existing key path are supplied on stdin. The private key remains in its existing
local file. Credentials, headers, token and APNs response body are absent from
the sender's output. APNs 200 is delivery-service acceptance only; it cannot
prove a device wake. Dependencies, accounts, keys and certificates are not
created by this helper.

## Offline qualification

Run with the established Python and Node executables. Pass the validator from
the exact frozen release source; its bytes must equal the local validator.
Use a new private output directory; retain failures as well as successful runs.

```sh
python3 -B scripts/ios_voip_background_wake.py qualify \
  --node /absolute/path/to/existing/node \
  --tagged-validator /absolute/path/to/tagged/scripts/ios_runtime_acceptance.py \
  --out /absolute/private/qualification-output
```

The tests use synthetic identities, logs and inventories. The real sender
function runs against fake transport, and real ES256 signing is checked with an
in-memory synthetic key. No test opens a network connection. Qualification
binds all helper/parser/test bytes, the unchanged v3 validator, the Node and
Python executables and the full test log. Requalify changed source or tools.
This is new independent qualification, not recovered historical qualification.

## Private inputs and review

The input object uses schema `iris-voip-background-input-v1` and exactly these
fields: `schema`, `window`, `node`, `credentials`, `token_evidence`,
`qualification`, `existing_pid`, `trace_transport`. Bindings have `path` and `sha256`; keep every
actual path, device identity, subscription and credential outside source control.

`window` is the existing `iris-relaunch-adapters-v1` admission, binding the
reviewed installed distribution and toolchain. Its direct parent must own both
native acceptance and exact-device leases. Use the established output/deadline
and free-space bounds (at most 100 MiB output and 600 seconds). Existing HID tool
bindings remain required by this shared admission but the wake route never
starts HID. `existing_pid` must be a fresh actual inventory of the admitted app.

`credentials` binds `iris-existing-apns-credentials-v1`: the production bundle,
environment, established `key_id`/`team_id`, owner-only `local_key` binding and
matching `remote_key_sha256`. Derive it from the existing production service
configuration and a hash-only comparison with the existing local key. Do not
export a key or create new signing material. `node` must match qualification.

`trace_transport` binds an `iris-ostrace-transport-v1` object with the existing
`site_packages` directory, `source_pins` and pinned `python` executable. The
maintained `ios_ostrace_capture.py` adapter uses the pinned upstream
`pymobiledevice3` framing and decoder without modifying its installation. Every
selected record is retained byte-for-byte in `records.bin`, indexed, and rendered
with its actual device seconds and microseconds in UTC. Optional legacy metadata
stays opaque. Unrelated multiline diagnostics render with escaped line breaks;
multiline startup/lifecycle events, malformed framing, lost-message notices and
disconnects fail the observation. No logger reconnects or retries.

The final `trace_capture` evidence binds both raw streams. Acceptance replay
re-renders every selected frame and compares the entire log with the measured
capture, in addition to the unchanged live cutoff and owned-process checks.

`token_evidence` binds `iris-owned-voip-token-v1` with the exact `candidate`,
`device`, `bundle_id`, `subscription_id`, `inputs` and a separate `review` binding.
Its `inputs` are bindings of:

- `identity_evidence`: a bounded list of actual evidence establishing that exact
  owned-phone subscription ID. A supported read of the known app Preferences
  plist (`settings.call_push_subscription_id.ios`) and raw copy command is one
  source. Previously retained owned-phone registration evidence is also usable
  when the owner can establish current continuity. Container access is not a
  release requirement.
- `phone_continuity`: a bounded list of actual phone/install/registration
  evidence connecting the owned registration to the current candidate. The
  separate review establishes what those facts prove; a historical token alone
  does not establish current ownership.
- `installed_inventory`: raw CoreDevice JSON with one exact candidate app.
- `subscription`: the raw existing server subscription JSON for that exact ID,
  containing one production token, the bundle's `.voip` topic, call kind 21111,
  and no alternative filters.
- `lookup`: `iris-exact-subscription-read-v1`, with `subscription_id`, the raw
  `response` binding, `read_only: true`, `record_count: 1`,
  `scope: "current-owned-phone-call-subscription"` and `observed_utc`.

Use the existing authenticated GET for the exact subscription ID when its owned
authentication material is available, or an independently qualified read-only
point lookup against the established server storage. Do not enumerate unrelated
users, guess an account authentication key, or update a subscription. Normalize
the lookup only from a real reviewed request/response;
its fields are not a substitute for raw ownership evidence. Both phone and
server evidence must satisfy the release owner's explicitly reviewed freshness
and continuity decision. No arbitrary token-age limit is a release requirement.

The separate `iris-owned-voip-token-review-v1` record has `decision: "accepted"`
and exactly the same candidate, device, bundle, subscription ID and inputs. It
binds `token_sha256` (SHA256 of the exact token text), `ownership_basis` of
`current-registration` or `retained-registration-continuity`, and finite
`valid_from_utc`/`valid_until_utc` boundaries. The actual server lookup and push
must fall inside that reviewed window. Choose and justify these boundaries from
the evidence and planned operation, rather than treating them as a tagged gate.
The release owner must review the raw copy/inventory/lookup and token ownership;
the helper cannot independently establish ownership from an ID or plist alone.

## One admitted physical execution

After qualification and owner review, the lease-owning parent may invoke:

```sh
python3 -B scripts/ios_voip_background_wake.py observe \
  --input /absolute/private/input.json --input-sha256 REVIEWED_SHA256 \
  --output /absolute/private/new-runtime-output --execute
```

The controller starts both loggers and verifies readiness, normally activates
Settings, observes the old app suspended, terminates only that admitted old PID
once, and proves actual absence with a fresh native inventory. It then sends
one request. No XCTest, debugger, app launch override or artificial app-state
delay is used during measurement. The OS must create one new process, confirmed
by one exact SpringBoard bootstrap, its executable, actual inventories and a
continuous loss-free lifecycle capture. Settings must stay foreground and Iris
must have zero foreground activations through suspension. Unexpected exits,
replacement, gaps, logging loss, unobserved tails and early logger closure fail.

The initial audit token is explicitly unavailable for this OS-created process.
The identity basis is the absent baseline, one OS bootstrap and uninterrupted
PID/executable/lifecycle evidence. Resume supplies a real audit token. Where an
initial token is available, the resume validator compares the actual tokens;
it never invents or claims an unavailable comparison. APNs payload-to-process
correlation is also not claimed.

The controller measures at least 135 seconds of actual suspension and resumes
once through ordinary CoreDevice launch, preserving the PID/executable and
observing at least 15 more live seconds. It retains complete raw logs, exact
live-sampled prefixes, the pre-signal cutoff and process-group closure. Only the
already qualified intentional terminal logger output is permitted after cutoff.
A failure is retained with partial samples and actual cutoff if one existed;
there is no automatic retry or fallback launch.

The output remains private evidence with `physical_release_acceptance: false`.
The release owner must review it, verify preserved state and crash results, and
compose the unchanged v3 receipt only after every required gate passes.

Protocol references: [Apple APNs provider connection and request format](https://developer.apple.com/library/archive/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/CommunicatingwithAPNs.html),
[Node HTTP/2](https://nodejs.org/api/http2.html), and
[Node crypto signature encodings](https://nodejs.org/api/crypto.html).
