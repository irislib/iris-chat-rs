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
| crates.io | Existing packages are outside this release process | Legacy |

The release builds Android arm64, iOS, macOS arm64, Windows x64, Linux x64,
and CLI archives for macOS arm64/x64 and Linux x64.

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

Local checks are optional and should be chosen according to the release risk:

```bash
just verify-fast
./scripts/test-release-gate --full
```

The hosted release workflow runs the authoritative release gate.

Release workflow and release-note edits run the short **Release checks**
workflow. Changes limited to Markdown, `.github/workflows/release.yml`, and
`scripts/test_release_workflow.py` do not restart the full native CI matrix.
Before tagging, require all nine jobs to succeed in a full CI run from `main`
with the default scope, or an explicitly selected `all` run. Skipped platform
jobs do not qualify. Record the run ID and source commit, then compare that
commit with the candidate: only the excluded paths above may differ. Any other
difference requires a new full CI run. When Release checks applies, require it
to succeed at the exact candidate commit and record that run ID too.
The tagged release still runs every release and mesh resource gate.

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
`--check`; Nostr publication runs only after the canonical Hashtree updater
readback succeeds. Any Haps failure fails distribution. No separate build,
catalog maintenance, or mobile installer execution is involved. Haps publication
uses the public relays in the dedicated Hashtree configuration, bypassing local
daemon preferences and inherited `NOSTR_RELAYS`; a local storage acknowledgment
is not proof of public delivery. Verify both packages from a fresh consumer
before treating Haps distribution as complete. The local publisher needs Haps
with `import-release` support and Python 3.9 or newer.

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
- Homebrew: update the tap and run `brew info iris`; its formula URL must
  contain the immutable release tag.
- Zapstore: confirm the version and publisher in Zapstore.
- Apple: workflow success means delivery/submission succeeded. Approval and
  public availability remain visible in App Store Connect.

Retry a failed GitHub job for the same tag; the workflow reuses an existing
immutable release and verifies every asset instead of modifying it. Local
channel commands and the iOS workflow are also safe to rerun with the same
tag. A retry never substitutes newly built bytes.

If a binary must change, commit the fix and create a new corrective tag.
Never reuse a tag or substitute a locally built artifact.
