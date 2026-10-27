# Release and Distribution Playbook

Iris Chat uses a build-once, distribute-many release model. GitHub Actions is
the only release builder. Apple workflows and local publishers promote the
exact, attested files from an immutable GitHub Release.

## Supported Platforms and Channels

| Channel | Content | How it is published |
|---|---|---|
| GitHub Release | Android, iOS, macOS, Windows, Linux, and CLI | Push a stable release tag |
| Hashtree + Haps | The complete GitHub Release asset set, update metadata, and signed desktop/CLI packages | `./scripts/distribute hashtree --tag <tag>` |
| Zapstore | The exact GitHub Android APK | `./scripts/distribute zapstore --tag <tag>` |
| Homebrew | The exact GitHub CLI archives through immutable Hashtree URLs | `./scripts/distribute homebrew --tag <tag>` |
| TestFlight | The exact GitHub IPA | Run **iOS Distribution** with `testflight` (internal) or `testflight-public` (public beta) |
| Apple App Store | The exact GitHub IPA | Run the **iOS Distribution** workflow with `app-store` |
| Google Play | A signed AAB is built, but upload is not automated | Roadmap |
| crates.io | Protocol library and CLI source packages | Publish verified source packages separately |

Haps publication also updates the source used by Omarchy's Haps menu. Users
enable that integration once with `haps omarchy`; no separate Omarchy release
or menu edit is needed. AUR is not currently a published Iris Chat channel:
local recipes alone do not establish a listing. Before enabling automatic AUR
publication, obtain a working AUR publisher identity and package access, then
promote the exact attested Linux release with pinned checksums and regenerated
`.SRCINFO`. Do not add a second application build or block existing channels on
AUR registration. Use a public company contact in package metadata and commits.

The release builds Android arm64, iOS, macOS arm64, Windows x64, Linux x64,
and CLI archives for macOS arm64/x64 and Linux x64.

The existing crates.io packages use separate semantic versions. When updating
`iris-chat-protocol`, advance its version and the exact local consumer pins and
lockfiles. Update the `iris-chat` package too when its published CLI would
otherwise retain an older protocol. Run the Rust gates and package verification,
publish the protocol first, then verify and publish the CLI source package.
Before either crates.io upload, push its exact source revision and ancestors to
the owned Hashtree source URL in `haps-release.json` and require a fresh,
complete-history verification with the pinned Haps verifier described below.
A verified descendant's complete history may cover an ancestor package revision
only with recorded ancestry and matching source configuration. Retain that
proof together with the verified package archive's checksum; a successful Git
push alone is insufficient.
Check the registry archives and dependency pins after publication. Do not use
the unsupported scripts under `scripts/legacy/release`. This source publication
does not replace any tagged binary or satisfy a binary distribution gate.

An explicitly authorized iOS omission is recorded for its exact tag in
`release-platforms.json` before tagging. All other tags still require the full
asset set. The policy permits only iOS exclusions: `v2026.10.8.2` omits the IPA
and Xcode archive from both GitHub and Hashtree. The tagged build workflow reads
that committed policy, so pushing the tag and manually retrying it have the same
scope. There is no mutable workflow flag or missing-file inference.

The attested manifest records the exclusion, and every distributor verifies it
against the tag policy while still requiring every remaining artifact. Keep
published policy entries unchanged so historical inventories stay verifiable.
Older full manifests without an exclusion field remain valid. iOS Distribution
rejects excluded tags before downloading an IPA or contacting Apple. This does
not skip any CI platform checks or the tagged core and mesh gates. iOS uses Apple
updates, while Hashtree desktop/CLI discovery retains every matching artifact;
publishing still preserves earlier version directories through the required
verified release-tree root.

The Linux CLI is built in Debian 12 so it does not inherit the hosted runner's
newer glibc requirement. Before uploading it, the build workflow installs the
exact archive in a fresh Debian 12 container as root and as a regular user,
then runs `iris --version` and `iris --help` for both installations. Test the
public website installer with `./scripts/test_cli_install_docker`.

## Publisher Identities

Public identities are fixed in the repository and checked before publication:

| Purpose | Expected public key | Local state |
|---|---|---|
| Hashtree releases and Homebrew | `npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap` | `~/.config/iris-chat/htree-nsec` |
| Zapstore | `npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm` | `~/.config/iris-chat/zapstore-nsec` |

The files must contain the matching secret keys and be readable only by their
owner. The distributor derives each public key before publishing. Hashtree
also verifies the active user in its dedicated configuration directory. There
is no shared signer variable and no browser-signing fallback.

Optional path overrides are `IRIS_HASHTREE_NSEC_PATH`,
`IRIS_HASHTREE_CONFIG_DIR`, `IRIS_HASHTREE_DATA_DIR`, and
`IRIS_ZAPSTORE_NSEC_PATH`.

Local distribution requires an authenticated GitHub CLI with `release verify`
support, plus `jq`, `python3`, and the channel tools:

- Hashtree: `htree` with `release publish --expected-root` support, `nak`, `curl`,
  and Haps with the `import-release` subcommand
- Homebrew: `htree`, `nak`, `curl`, and `git`
- Zapstore: `zsp`, `nak`, `base64`, and `sed`

Hashtree uses `~/.config/iris-chat/htree-release-config` for configuration and
`~/.config/iris-chat/htree-release-data` for stored content. Provision the
configuration directory's `keys` file with the same identity stored in
`IRIS_HASHTREE_NSEC_PATH`, using directory mode `0700` and file mode `0600`.
The distributor scopes both directories to its Hashtree and Homebrew commands,
regardless of inherited `HTREE_CONFIG_DIR` or `HTREE_DATA_DIR` settings.

Identity comes from `HTREE_CONFIG_DIR/keys`; setting only `HTREE_DATA_DIR` does
not isolate it. Confirm the dedicated identity before publishing:

```bash
HTREE_CONFIG_DIR="${IRIS_HASHTREE_CONFIG_DIR:-$HOME/.config/iris-chat/htree-release-config}" \
HTREE_DATA_DIR="${IRIS_HASHTREE_DATA_DIR:-$HOME/.config/iris-chat/htree-release-data}" htree user
```

## 1. Prepare the Release

Add a section to `RELEASE_NOTES.md`:

```markdown
## v2026.7.28

### GitHub

- Technical release summary.

### Apple

- Concise customer-facing changes.

### Zapstore

- Concise customer-facing changes.
```

The release tag is the actual release date: `vYYYY.M.D`. If another immutable
build is needed on the same date, use `.1`, `.2`, and so on. Never move or
replace a published tag.

Update both package versions and every build version argument in `haps.toml`
for the same tag. Check all six source package variants with
`python3 scripts/validate-haps-manifest.py --tag <tag>` before dispatching CI.
The Linux Haps source recipe must invoke locked Cargo directly and stage its
matching output. Do not route it through `scripts/linux-release`: that separate
Docker/DEB packaging pipeline provisions additional build tools. The validator
rejects that substitution. An audit of an older published recipe must be rerun
against the newly published package; a source change does not clear old findings.

Choose pre-release checks according to the changed components and release risk:

```bash
just verify-fast
./scripts/test-release-gate --full
```

Pre-release validation can run locally, on hosted CI, or a combination of both.
Keep the actual source revision, commands, toolchain, results, and artifact
checksums with each result. A full successful native CI run is convenient
coverage, but its execution location is not a release requirement.

Reuse successful checks when the relevant production code, tests, dependencies,
and build inputs are unchanged. Record the exact compared inputs and original
tested revision; do not relabel an older result as a new build. Run focused
regressions and the affected platform checks for changed inputs. A change to
one desktop view does not require rebuilding unrelated mobile platforms or
repeating unchanged device-link, web, and protocol checks. Shared-core changes
need coverage for their affected consumers and physical paths. Preserve failed
results; skipped checks and infrastructure failures do not count as passes.

For quick Rust iteration, set `CARGO_TARGET_DIR` to the established compatible
cache (explicitly in a worktree), reserve one writer for that cache throughout
the run, then use
`scripts/test-rust-focused --crate core --test module::exact_test --output work/focused-check`.
Select `core`,
`chat-protocol`, or `protocol-ffi`, and repeat `--test` for additional exact names.
The runner first checks the selected crate with `cargo fmt --check`, stopping
before compilation on failure and retaining `format.log`. It then builds one
library test binary, uses two jobs and locked offline
dependencies, and rejects zero-match or ignored-only results. `--dry-run` shows
the target and commands without invoking Cargo; `--online` explicitly permits
dependency fetching. Use `--release` and `--include-ignored` only when the selected
tests require them. The default timeout is ten minutes; optional
`--min-free-gib` and `--max-growth-gib` stop on a low free-space reading or
cumulative filesystem growth, including other writers on that filesystem.
Preserve the output directory containing full logs and
source/toolchain/binary/result provenance.
The existing CI workflow also accepts `scope=rust-focused`, `focused_crate`,
and `focused_tests` (whitespace-separated exact names), with optional
`focused_release` and `focused_ignored`. It retains compiled dependencies even
after a test failure and reuses them across source revisions. These focused
checks leave the full validation and tagged release gates unchanged.
The tagged core gate also retains compiled outputs after a test failure, using
the existing target paths and default-branch cache fallbacks. It keeps at most
one partial and one complete cache per source, compiler, architecture and lock
set. Repeated failures reuse the first partial cache; a later passing gate saves
the complete cache without creating a new multi-gigabyte entry for every retry.

For an optimized local Mac development build, reserve the native host and one
writer for the selected caches, set `CARGO_TARGET_DIR` to the existing absolute
core cache, and run `scripts/macos-build macos-dev`. Select the existing bindgen
cache with `IRIS_MACOS_BINDGEN_TARGET_DIR` and optionally a stable Xcode cache
with `IRIS_MACOS_DEV_DERIVED_DATA`. This builds only the native Rust library,
once, with locked offline dependencies and two jobs by default; both Rust and
Swift use release optimization, with the build channel set to `development`.
Unchanged bindings and the Xcode cache survive
repeated builds. The printed app path is for development verification: this
command does not sign for distribution, install, launch, or publish it. A first
optimized build may need a cold cache; keep bounded time and disk limits around
it. Local performance evidence does not replace required acceptance of the
unchanged tagged GitHub release artifact.

Run local CPU checks separately from compilation, using the same verified app
bundle and a bounded runtime lease. Open that exact bundle through Finder's
normal Open action, then confirm its executable, process start identity, and
foreground status before starting the settling period. Accessibility window
raises and clicks can reach a background app without activating it. A visible
window alone does not establish foreground readiness. Keep the Mac unlocked
and foreground checks active throughout measurement; if focus changes, retain
the failed result. Quit normally afterward and verify the process and leases
have closed before admitting other native work. A launch or focus failure
does not require rebuilding unchanged inputs.

For version-only package verification, dispatch `scope=rust-package` with
`package_crate=chat-protocol` or `core`. This runs locked package verification
with two build jobs and retains the archive, checksum, logs and exact source;
it does not publish. Verify and publish the protocol first so core's registry
dependency resolves to that version, then verify the core package.

Release workflow and release-note edits use the short **Release checks**
workflow or the equivalent local checks at the candidate revision. Changes
limited to Markdown, `.github/workflows/release.yml`, and
`scripts/test_release_workflow.py` do not restart the full native CI matrix.
Before tagging, retain a coverage record connecting the final source to each
required result, including any unchanged-input reuse. A focused pass does not
stand in for unrelated platform coverage or a required physical-device check.

The tagged GitHub release workflow remains the release builder and runs every
release and mesh resource gate before publishing its signed, attested assets.
Public release also requires the exact source commit and all ancestors to be
available from the owned Hashtree source URL in `haps-release.json`. The
maintainer publishes source with the existing signing identity; CI only reads
it. Before attestation or public GitHub release creation, CI checks the tag,
validated source and manifest agree, then uses the pinned Haps verifier and
checksum-pinned helpers for a fresh fetch and complete-history traversal.
Missing commits, shallow or incomplete history, and verification failures stop
publication. No signing key is added to CI.

Bluetooth changes also require a physical iPhone/Android check before release.
`--on-device` checks LAN visibility; it does not exercise Bluetooth, and the
opt-in `FipsBlePhysicalUITests` skip without `IRIS_FIPS_PHYSICAL_PEER_NPUB`.
Use isolated test accounts, enable the receiving test account's read receipts,
disable its message servers and IP networking, and leave Bluetooth enabled.
Require the exact message, a return receipt, and the `FIPS nearby` transport
trace; restore the original network settings afterward. Keep the device logs
and test result locally with the tested commit. A simulator pass is not a
physical Bluetooth result.

Device-linking or history-transfer changes also require the web repository's
`pnpm test:public-device-link` against the intended native development build and
web candidate. Set `IRIS_CHAT_RS_DIR` and the explicitly selected physical
`IRIS_LINK_TEST_UDID`; use `IRIS_LINK_TEST_URL` for a candidate preview. Keep the
default public message/FIPS servers. Require successful approval plus a pre-link
message appearing exactly once after browser reload, and record both source
commits with the private evidence. Repeat against the deployed web release.
Local loopback interop and a phone's new device entry alone do not pass this gate.
Also run `python3 scripts/test_public_native_device_link.py` with the same
explicit phone and intended native CLI (`--binary` can select its build).
Require that pre-link history arrives exactly once and survives a native
service restart. Both physical tests preserve ordinary account storage.

The hosted workflow also runs the pinned Iris Stack process gate against the exact tagged Chat
commit and known-good public Drive/Hashtree versions. Relayless recovery, CPU,
and bandwidth checks, including two 65-second idle windows covering periodic
maintenance, must pass before GitHub release publication; their receipt
remains a workflow artifact and is excluded from the app release files.

The mandatory Rust suite also restores a generated 100 MiB account database with
96 contacts, three devices per contact, three own devices and eight groups,
plus 96 separate live-endpoint discovery scopes. The ordinary core suite runs
the one-relay populated-discovery test once. After that suite, the runner uses
the existing target cache to run the optimized five-relay variant exactly once,
covering all 2,400 author/relay pairs, including live-endpoint discovery.
Both variants wait for every author/relay pair to settle, then require two
35-second windows below 5% of one CPU core with no repeated application
requests. Two further 125-second windows cover actual recurring discovery polls
for every pair, each below 5% of one CPU core, followed by prompt targeted proof
recovery on every relay. TLS sessions stay isolated by author and exact
message-server URL; a quiet gap alone does not satisfy the CPU gate. Signed wire
checks must keep each peer separate from account receiving and authentication.
Dispatch received events through the app's production batch handling throughout
settling and measurement. Each measured window also replays the fixture's signed
rosters and invites after eviction from the recent-event and signature caches;
require completed admission with no app-row writes or protocol storage mutations.
Targeted recovery must apply its fresh proof, not merely receive its network
frame. The fixture retains its isolated FIPS transport and does not measure full
production FIPS reconciliation; physical CPU acceptance remains required.
Count completed request/close pairs independently of connections, with per-relay
counters matching the process totals. Each cycle must exercise all 64 retained
connections across the whole process, not 64 per relay, and the remaining
disposable readers, whose new connections require scoped TLS resumption and
fresh anonymous authentication. Connection reuse must not hide missed polls or
extra traffic. Retain both CPU/wire receipts and full child logs with the
private-control test artifact. The relayless resource fixture alone does not
cover this workload.

The Rust suite also exercises a 254,672,896-byte protocol checkpoint with 22,702
queued group repairs and 19,231 encrypted group messages. Small incoming and
outgoing batches must reuse unchanged payload serialization, commit once and
write less than 4 MiB of the existing checkpoint. Failed saves and reopening
must preserve pending work and the exact ratchet state. This mandatory storage
regression complements physical CPU checks; it does not establish a device's
CPU usage. SQLite stores immutable queued payloads separately from their order
and retry metadata, committing them with the ratchet state in one transaction.
Growth, queue append and retry retirement tests measure actual SQLite writes;
unchanged saves must write nothing. Existing JSON checkpoints migrate on their
first save, and read-only recovery still reconstructs the same logical state.
Incoming and outgoing batch tests also measure SQLite page reads after an
app-state write and page-cache eviction, so small query results cannot conceal
repeated traversal of immutable payload pages. Compact chunk metadata is
maintained transactionally; full reads still validate the original chunks.
Authorization replay tests must keep unchanged-event allocations independent of
the pending backlog, avoid unnecessary saves, and preserve exact evidence,
device reconciliation and rollback of real authorization changes.

On Linux, the same runner also requires exactly one optimized
`restored_backlog_batched_retry_stays_within_cpu_budget` test. It restores a
350,224,384-byte SQLite checkpoint with 4,214 pending incoming packets, 24,990
group fanouts, 23,437 encrypted group messages and 44 outgoing retries. Each
retry turn includes the app's batch boundary, journal acknowledgment and durable
flush. Continuations have a 25 ms minimum delay and pace blind history recovery
from its measured work; liveness checks run every 2 seconds. The fixture uses
16 authors, 46 sender-key states and 30 key distributions, with encrypted payloads
ranging from 64 to 16,384 bytes. Restore,
scheduler readiness and the valid message must complete within the three
60-second settling slices. Two further 60-second slices must each use at most
5% of one CPU core while exercising recurring due work and durable saves.
Two additional expiring controls retire in separate measured minutes, and each
retirement must flush in its own retry turn. All 44 unresolved outgoing messages
remain queued. Backoff-only retries must not substitute for real durable work.
Unresolved work must survive, unchanged inputs must not restart exhausted
searches, and a reload must preserve queues and ratchets. Missing-key searches
may still make finite progress during measurement.
Fresh current-position messages must bypass a waiting history search without
replacing its cursor. Keep the signed-event regression with 2,000 retained keys
and multiple key generations below one second of virtual arrival-to-delivery
time, including measured decryption and pacing, and verify that the app replaces
a later history timer when fresh work becomes eligible sooner.
Incoming packets must retain valid NIP44 encodings and fail authentication,
with positive controls proving the unmodified packets decrypt. Check their
format and decoded sizes against the recorded aggregate workload.

The gate uses release optimization level 3, debug info level 1 and disabled
debug assertions, with two build jobs and the existing Cargo target/cache.
Retain its complete stdout, including all five `batched_retry_slice` records and
the `iris-batched-retry-cpu/v4` summary. The runner rejects missing execution,
incomplete measurements or a CPU failure. Keep standalone retry diagnostic
results and the physical-device CPU gates as separate evidence.

## 2. Tag the Latest `main`

Merge the release notes and all intended changes first. Then:

```bash
git switch main
git pull --ff-only github main
test "$(git rev-parse HEAD)" = "$(git rev-parse github/main)"
test -z "$(git status --porcelain)"
git tag -a v2026.7.28 -m "Iris Chat v2026.7.28"
git push github v2026.7.28
```

Stop if the tree is not clean or the two commit IDs differ. Pushing the tag
starts the **Release** workflow. It validates the notes and tag, builds the
platforms required by the tag policy, gives every binary one versioned filename,
creates a digest manifest, attests every file, and publishes the GitHub Release.

Wait until that workflow succeeds before distributing anywhere else. A
successful GitHub Release means the binaries exist; it does not mean the store
or local channels are published.

## 3. Publish Local Channels

Run one channel at a time. `--check` downloads and verifies the exact assets,
manifest, attestations, and signer without publishing:

Before Hashtree publication, resolve the existing release-tree CID through
signed discovery and independently compare it with the refreshed public gateway.
Inspect that root's version entries, then export it as
`IRIS_HASHTREE_EXPECTED_ROOT`. Publication requires this explicit root so a
missing live lookup cannot replace the existing release history. A conflicting
root stops publication; verify and capture the current root again before retrying.

```bash
./scripts/distribute hashtree --tag v2026.7.28 --check
./scripts/distribute hashtree --tag v2026.7.28
```

Hashtree distribution also publishes the same desktop/CLI bytes through Haps,
using `haps-release.json` and the existing dedicated Hashtree signing identity.
The Haps checksum/layout/signer check runs before publication, including with
`--check`. Real promotion first uses `import-release --publish-source` to push
the verified manifest's source commit and its ancestors to the owned source
URL under `refs/tags/haps-source/<commit>` and verify a cold fetch. Failure,
including an unsupported flag, stops before binary upload or release-tree
publication. Package metadata publication runs only after the canonical
Hashtree updater readback succeeds. Any Haps failure fails distribution. No
separate build, catalog maintenance, or mobile installer execution is involved.
Haps publication uses the public relays in the dedicated Hashtree configuration, bypassing local
daemon preferences and inherited `NOSTR_RELAYS`; a local storage acknowledgment
is not proof of public delivery. Verify both packages from a fresh consumer
before treating Haps distribution as complete. The local publisher needs Haps
with qualified `import-release --publish-source` support and Python 3.9 or newer.
Set `IRIS_DISTRIBUTION_TOOLS_DIR` to an absolute directory containing a qualified
`haps` and `htree` pair to use dedicated release tools without replacing tools
used by other local services. Haps helpers with source-file caching and
retained-root delta uploads avoid processing all previous package payloads on
each release; older helpers report when these optimizations are unavailable.

For an already tagged release, run the reviewed current distributor checkout
with the explicit original `--tag`; record the tooling commit separately from
the release manifest's source commit. Do not restore the older tagged distributor,
move the tag, substitute the tooling source for the manifest source, or rebuild
the release assets to pick up a publication-tooling fix.

Use this order: Hashtree, Homebrew, Zapstore. Homebrew refuses to publish
until the exact Hashtree tag exists. Commands are safe to retry with the same
tag and bytes; they never select a tag, workflow run, or local build
implicitly.

Hashtree promotion also runs the attested CLI from that release with a fresh
data directory and the shipped update settings. Both CLI and native-app checks
must resolve the requested tag through signed discovery. An HTTP gateway
readback alone cannot pass promotion. Generated update metadata is tested
against the app's actual updater library, including same-day revision tags.

After Hashtree publication, the isolated Linux check below is also required
before continuing to Homebrew or Zapstore or calling the release complete.
A fresh data directory on the publisher's computer can still discover its
same-host provider through loopback, so the native check does not establish
remote-client discovery.

Use the exact attested Linux x64 CLI archive from the immutable GitHub Release,
extracted without installing or rebuilding it. Docker must support Linux x64
containers, including emulation on ARM hosts:

```bash
release_tag=v2026.7.28
release_check="work/updater-check-${release_tag}"
mkdir -p "$release_check"
gh release download "$release_tag" --repo irislib/iris-chat-rs \
  --pattern "iris-${release_tag}-x86_64-unknown-linux-gnu.tar.gz" --dir "$release_check"
archive="$release_check/iris-${release_tag}-x86_64-unknown-linux-gnu.tar.gz"
gh attestation verify "$archive" --repo irislib/iris-chat-rs --source-ref "refs/tags/$release_tag"
gh release verify-asset "$release_tag" "$archive" --repo irislib/iris-chat-rs
tar -xzf "$archive" -C "$release_check"
python3 scripts/check-release-updater.py --isolated-linux \
  --cli "$release_check/iris/iris" --tag "$release_tag" \
  --receipt "$release_check/updater-proof.json"
```

This runs CLI and app checks in separate read-only Ubuntu containers with
fresh bridge network namespaces and temporary profiles. It mounts only the
executable and a CA bundle, uses shipped discovery settings, and requires a
verified result for the exact tag in both modes. A signed result through any
healthy configured message server is sufficient; another server being
unavailable does not invalidate that result. No discovery overrides, host
network, account data, or publisher configuration are passed to the checks.
If the host has no standard CA bundle, pass `--ca-file /path/to/ca.pem`.
Preserve the JSON receipt with the release evidence. A failing isolated check
blocks completion even when the native check succeeds.

After the isolated check passes, publish the remaining local channels:

```bash
./scripts/distribute homebrew --tag "$release_tag" --check
./scripts/distribute homebrew --tag "$release_tag"
./scripts/distribute zapstore --tag "$release_tag" --check
./scripts/distribute zapstore --tag "$release_tag"
```

Zapstore requires a current C1 ownership proof for the APK's signing certificate.
The **Zapstore certificate proof** workflow uses the established Android release
credentials to produce only a public certificate and unsigned proof; it does not
rebuild the APK or export the private key. Sign and publish that proof with the
dedicated Zapstore publisher before retrying a missing or expired proof.

## 4. Publish to Apple

In GitHub, open **Actions → iOS Distribution → Run workflow**.

- The `ios-app-store-release` environment must provide `ASC_PRIVATE_KEY_P8`,
  `ASC_KEY_ID`, and `ASC_ISSUER_ID`.
- Set `IRIS_IOS_BUNDLE_ID` when its default is not sufficient. TestFlight
  requires comma-separated internal group names in `IRIS_TESTFLIGHT_GROUPS`.
- Enter the exact stable tag.
- Choose `testflight` to upload or reuse that build and attach it to the
  configured internal TestFlight groups.
- Choose `testflight-public` to upload or reuse that same build, submit Beta App
  Review when required, and attach it to the existing public external groups
  named by `public_groups` (default `Public`). Their public links must already
  be enabled. This does not submit an App Store release or expire older builds.
  Automatic distribution after Beta App Review approval is enabled. An already
  approved build is activated for testing, including when retrying distribution.
  The workflow reports whether the build is available or still awaiting review.
  Before calling the public release complete, verify the exact build is in the
  public group with external status `IN_BETA_TESTING`; approval alone is not
  availability. Confirm it is offered through the group's public TestFlight link.
- Choose `app-store` to upload or reuse that build, apply the Apple notes, and
  submit it for review.
- For App Store, choose automatic, manual, or phased release after approval.
  Automatic is the default.
- Every iOS App Store release must advance the Apple-visible
  `CFBundleShortVersionString`; changing only the fourth/internal component cannot trigger update discovery.
  Starting with `v2026.9.8.1`, Apple's third component is `day * 100 + revision`:
  `v2026.9.8.1` becomes `2026.9.801` and `v2026.9.9` becomes `2026.9.900`.
  Older tags keep their original Apple versions for immutable artifact verification.

The workflow verifies the tagged IPA and its attestation before contacting
App Store Connect. Retrying does not rebuild or upload a duplicate build.

For each new release, before `testflight-public` or `app-store`, the release owner
must verify the exact release on a reserved physical iPhone. Use the actual
TestFlight upgrade route, or the explicitly user-authorized production AdHoc
route below. Internal TestFlight upload and processing may happen first.
An archive, simulator run, development install, or successful launch command
does not establish physical release acceptance.

Before preparing device-specific signing, match the intended phone's exact UDID
against the current USB/CoreDevice inventory, within existing device-access holds.
Include that ID in the resource reservation, profile, installation and measurement
receipts. Recheck connectivity at the start of the reserved window.

Use an owned test account populated by the previous installed version. Update
it in place through the selected route, preserving its app data and Keychain. Do not
erase data, restore a secret key into a fresh profile, or use automation storage
overrides. Record the previous and installed version/build, device and OS, tagged
source, immutable IPA and manifest hashes, and App Store Connect build mapping
for a TestFlight upgrade.
Keep this private runtime evidence with the release coverage record.

For `authorized-production-adhoc`, retain the explicit direct-install authorization
and derive the private proof from qualified, hash-bound release, original-signing,
preparation, provisioning, installation and ordinary-launch receipts. Use only the
official tagged IPA. Verify unchanged instruction content, load commands and UUID
for the app and both extensions; keep the original IPA unchanged. Re-sign only for
the reserved device with production APNs and debugging disabled. The only allowed
entitlement removal is `beta-reports-active`. No uninstall, profile restore, data
reset or Keychain reset is permitted. This route verifies an in-place production
AdHoc upgrade; it does not claim a TestFlight upgrade, extraction of the installed
binary, or byte-identical signatures after re-signing. Both routes require every
physical lifecycle, crash and quality check below.

Launch normally without a debugger or XCTest launch assertion. Verify the
account, an existing message and a draft; observe the same process for at least
60 seconds in the foreground and 135 seconds in the background, then resume and
cold relaunch and verify the preserved state again. A crash corrective release
must also exercise its reported failing flow and focused regression.

Also exercise startup in the background and backgrounding during account
restoration. For `cold_background_launch`, start a new process while another app
remains active; record background entry and zero foreground activations through
suspension. For `early_background`, launch normally and switch away while account
restoration is still in flight. Capture evidence of that overlap; waiting until
restoration finishes does not cover this path. For each, retain the original
process from launch through at least 135 seconds of actual background suspension,
without a debugger or XCTest launch assertion. The 135-second interval starts
after suspension, not at launch. Verify preserved state after resuming.
Use the startup lifecycle evidence and focused regression to establish these
conditions; a successful warm-background check is insufficient. A background
`affected_flow` must likewise survive at least 135 seconds suspended.

Preserve the first failure and diagnostics; an automatic relaunch or retry is not a pass.
A missing device, unverified installed build, or unavailable crash reproduction
remains an explicit outstanding gate. Do not promote to public TestFlight or
submit to the App Store while that gate is outstanding.

After verification, retain the private captures and prepare a safe receipt using
the `ios-release-upgrade-v3` schema in `scripts/ios_runtime_acceptance.py`.
It records the exact candidate, previous and installed builds, preserved-state
results, measured lifecycle intervals, process/crash counts, and private evidence
digests. It must contain no account or device identifiers, local paths, screenshots,
or raw logs. A success flag or the 15-second development-restoration check cannot
satisfy this contract. This validator rejects v1/v2 receipts; historical immutable
tags retain their own unchanged validators and previously valid evidence. The two startup checks include
`entry_state: "background"` and `foreground_activations: 0` for `cold_background_launch`, and
`restore_in_flight: true` for `early_background`, in addition to the common phase
fields. These attest to the private evidence; they must not be inferred from
process survival alone. Set `affected_flow.process_state` to the actual observed
state (`foreground` or `background-suspended`).

The v3 `production_adhoc` proof contains only fixed policy enums and evidence
digests. Derive it from the qualified private checks, not hand-written success
claims. Its three `bundle_instruction_sha256` entries hash the UTF-8 JSON object
containing `uuid`, `loadCommandsSHA256` and `unsignedContentSHA256` from each
verified code-binding record, with sorted keys, compact separators and no final
newline, for the app, notification service and share extension. Keep signing identities,
provisioned-device identifiers and private paths out of the safe receipt.

Both routes also require candidate-bound `quality` evidence. Use the qualified
installed-app observer: 180 seconds of foreground settling, one discarded
60-second interval and two retained 60-second intervals. Each retained interval
must be at most 5% of one CPU core and 5 MiB/minute of disk writes, with nominal
thermal status, unchanged process and complete foreground observations. Preserve
the evaluator's existing clock tolerances; a short metric interval alone does not
qualify without its matching observed interval. Do not average away a failed
interval, subtract unqualified observer overhead or retry unchanged failures.
Retain the qualified evaluator and source-input digests with the CPU receipt.

For startup, retain observations covering cold start through preserved state,
the focused login/loading regression and a release-owner review; an observed
login flash fails. For sending, retain measured send-to-visible time and the
release-owner review of the focused flow. The safe receipt records the measurement
and accepted review without inventing a latency threshold. Review digests bind
these decisions to the exact candidate and private observations; a status string
alone does not replace that evidence. Any known unresolved startup or sending
regression keeps promotion blocked.

Run **iOS Runtime Acceptance** from the exact release tag, supplying that tag and
the safe receipt JSON. It verifies the immutable release files, validates and
normalizes the record, attests its provenance, and retains the
`ios-runtime-acceptance` artifact. The workflow records the release owner's
physical verification; it does not run the off-machine phone tests itself.
Supply its successful run ID as `runtime_acceptance_run` when running
`testflight-public` or `app-store`. Promotion verifies the exact workflow,
repository, source, attestation, IPA and manifest hashes, Apple version/build,
and every required runtime result before configuring Apple credentials.

Keep the normalized receipt with the private evidence. Valid acceptance has no
age expiry for unchanged inputs. If a GitHub artifact expires, the same retained
receipt may be recorded again against the same tag without repeating valid
physical checks. Changed candidates or superseded acceptance contracts need
matching evidence; do not relabel a previous candidate's result.

## Verification and Recovery

- GitHub: the Release must contain the versioned manifest and all expected
  assets, with no `latest` aliases. `gh release verify v2026.7.28
  --repo irislib/iris-chat-rs` must succeed and the release page must say
  **Immutable**.
- Hashtree: the distributor refreshes the gateway's cached root and verifies
  the public `release.json` tag and commit against the GitHub manifest before
  reporting success. The attested Linux x64 CLI must also pass the isolated
  default-settings CLI/app check above. A failed readback can be retried with
  the same tag.
- Homebrew: update the tap and run `brew info --formula sirius/iris/iris`;
  its formula URL must contain the immutable release tag.
- Zapstore: the distributor verifies the public signed app/release/APK records
  and streams the full public APK to check its size and hash before reporting
  success. A publisher exit code or relay acknowledgement alone is insufficient.
- Apple: workflow success means delivery/submission succeeded. Approval and
  public availability remain visible in App Store Connect.

Retry a failed GitHub job for the same tag; the workflow reuses an existing
immutable release and verifies every asset instead of modifying it. Local
channel commands and the iOS workflow are also safe to rerun with the same
tag. A retry never substitutes newly built bytes.

If a binary must change, commit the fix and create a new corrective tag.
Never reuse a tag or substitute a locally built artifact.
