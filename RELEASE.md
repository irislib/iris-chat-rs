# Release and Distribution Playbook

Iris Chat uses a build-once, distribute-many release model. GitHub Actions is
the only release builder. Apple workflows and local publishers promote the
exact, attested files from an immutable GitHub Release.

## Supported Platforms and Channels

| Channel | Content | How it is published |
|---|---|---|
| GitHub Release | Android, iOS, macOS, Windows, Linux, and CLI | Push a stable release tag |
| Hashtree | The complete GitHub Release asset set and update metadata | `./scripts/distribute hashtree --tag <tag>` |
| Zapstore | The exact GitHub Android APK | `./scripts/distribute zapstore --tag <tag>` |
| Homebrew | The exact GitHub CLI archives through immutable Hashtree URLs | `./scripts/distribute homebrew --tag <tag>` |
| TestFlight | The exact GitHub IPA | Run the **iOS Distribution** workflow with `testflight` |
| Apple App Store | The exact GitHub IPA | Run the **iOS Distribution** workflow with `app-store` |
| Google Play | A signed AAB is built, but upload is not automated | Roadmap |
| crates.io | Existing packages are outside this release process | Legacy |

The release builds Android arm64, iOS, macOS arm64, Windows x64, Linux x64,
and CLI archives for macOS arm64/x64 and Linux x64.

## Publisher Identities

Public identities are fixed in the repository and checked before publication:

| Purpose | Expected public key | Local state |
|---|---|---|
| Hashtree releases and Homebrew | `npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap` | `~/.config/iris-chat/htree-nsec` and `~/.config/iris-chat/htree-release-data` |
| Zapstore | `npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm` | `~/.config/iris-chat/zapstore-nsec` |

The files must contain the matching secret keys and be readable only by their
owner. The distributor derives each public key before publishing. Hashtree
also verifies the active user in its dedicated data directory. There is no
shared signer variable and no browser-signing fallback.

Optional path overrides are `IRIS_HASHTREE_NSEC_PATH`,
`IRIS_HASHTREE_DATA_DIR`, and `IRIS_ZAPSTORE_NSEC_PATH`.

Local distribution requires an authenticated GitHub CLI with `release verify`
support, plus `jq`, `python3`, and the channel tools:

- Hashtree: `htree` and `nak`
- Homebrew: `htree`, `nak`, `curl`, and `git`
- Zapstore: `zsp`, `nak`, `base64`, and `sed`

Initialize the dedicated Hashtree data directory once with the same Hashtree
identity stored in `IRIS_HASHTREE_NSEC_PATH`. Confirm it before publishing:

```bash
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
starts the **Release** workflow. It validates the notes and tag, builds all
platforms, gives every binary one versioned filename, creates a digest
manifest, attests every file, and publishes the GitHub Release.

Wait until that workflow succeeds before distributing anywhere else. A
successful GitHub Release means the binaries exist; it does not mean the store
or local channels are published.

## 3. Publish Local Channels

Run one channel at a time. `--check` downloads and verifies the exact assets,
manifest, attestations, and signer without publishing:

```bash
./scripts/distribute hashtree --tag v2026.7.28 --check
./scripts/distribute hashtree --tag v2026.7.28

./scripts/distribute homebrew --tag v2026.7.28 --check
./scripts/distribute homebrew --tag v2026.7.28

./scripts/distribute zapstore --tag v2026.7.28 --check
./scripts/distribute zapstore --tag v2026.7.28
```

Use this order: Hashtree, Homebrew, Zapstore. Homebrew refuses to publish
until the exact Hashtree tag exists. Commands are safe to retry with the same
tag and bytes; they never select a tag, workflow run, or local build
implicitly.

## 4. Publish to Apple

In GitHub, open **Actions → iOS Distribution → Run workflow**.

- The `ios-app-store-release` environment must provide `ASC_PRIVATE_KEY_P8`,
  `ASC_KEY_ID`, and `ASC_ISSUER_ID`.
- Set `IRIS_IOS_BUNDLE_ID` when its default is not sufficient. TestFlight
  requires comma-separated internal group names in `IRIS_TESTFLIGHT_GROUPS`.
- Enter the exact stable tag.
- Choose `testflight` to upload or reuse that build and attach it to the
  configured internal TestFlight groups.
- Choose `app-store` to upload or reuse that build, apply the Apple notes, and
  submit it for review.
- For App Store, choose automatic, manual, or phased release after approval.
  Automatic is the default.

The workflow verifies the tagged IPA and its attestation before contacting
App Store Connect. Retrying does not rebuild or upload a duplicate build.

## Verification and Recovery

- GitHub: the Release must contain the versioned manifest and all expected
  assets, with no `latest` aliases. `gh release verify v2026.7.28
  --repo irislib/iris-chat-rs` must succeed and the release page must say
  **Immutable**.
- Hashtree: open the exact tag URL printed by the distributor and confirm
  `release.json` reports the requested tag.
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
