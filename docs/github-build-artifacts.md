# GitHub Artifact Builds

Status: implemented in the worktree but not yet run on GitHub.

## Scope

`.github/workflows/build-artifacts.yml` builds and stores:

- signed Android APK and AAB
- signed iOS IPA and zipped xcarchive
- unsigned Windows installer and portable ZIP, matching existing behavior
- Linux DEB and tarball
- CLI archives for macOS ARM64, macOS x64, and Linux x64

The macOS GUI app, DMG, updater archive, Developer ID signing, and notarization
are deliberately excluded. The existing local macOS scripts remain unchanged.

The workflow never creates tags or GitHub Releases, uploads to TestFlight or an
app store, publishes to Hashtree, Zapstore, Homebrew, or crates.io, or updates a
`latest` pointer. Its repository permission is `contents: read`.

## Invocation

The manual workflow accepts only:

- `ref`: branch, tag, or commit to build
- `artifact_retention_days`: 7, 30, or 90

There is no version override. The metadata job resolves `ref` to one immutable
commit before any platform job starts.

## Version Contract

The selected commit is the sole source of version identity:

- version name: the commit's UTC date as `YYYY.M.D`
- version code: `year * 10000 + month * 1000 + day * 100`
- build timestamp: the commit timestamp in UTC
- build SHA: the selected full commit plus its 12-character short form

For example, a commit from 2026-07-14 has version `2026.7.14` and version code
`20268400`. Every job receives these values from the metadata job. Rebuilding
the same commit produces the same identity regardless of dispatch date.

## Workflow

```mermaid
flowchart TD
    Trigger["Manual build: ref + retention"] --> Metadata["Resolve immutable commit and automatic version"]

    Metadata --> Android["Android artifact: signed APK + AAB"]
    Metadata --> IOS["iOS artifact: signed IPA + xcarchive"]
    Metadata --> Windows["Windows artifact: installer + portable ZIP"]
    Metadata --> Linux["Linux artifact: DEB + tarball"]
    Metadata --> CLI["Three CLI artifacts: macOS ARM64/x64 + Linux x64"]
```

### Metadata

The metadata job checks out the requested ref and exposes its commit, version,
and timestamp as job outputs. All later checkouts use the resolved commit rather
than the movable ref.

### Android

Runs on `ubuntu-24.04` using the existing `android-release` environment and
`scripts/android-release release-artifacts`. It verifies the APK with
`apksigner` and the AAB with `jarsigner` before uploading both files.

Required environment secrets:

- `IRIS_RELEASE_KEYSTORE_BASE64`
- `IRIS_RELEASE_KEYSTORE_PASSWORD`
- `IRIS_RELEASE_KEY_ALIAS`
- `IRIS_RELEASE_KEY_PASSWORD`

### iOS

Runs on `macos-15` using the existing archive/export implementation in
`scripts/ios-release`. It verifies the signed app and stores the IPA and zipped
xcarchive. It never invokes the upload or TestFlight commands.

Required secrets and variables:

- `ASC_PRIVATE_KEY_P8`, `ASC_KEY_ID`, and `ASC_ISSUER_ID`
- `IRIS_IOS_BUNDLE_ID` and `IRIS_IOS_DEVELOPMENT_TEAM`

These should be scoped to a protected `ios-build` environment.

### Windows

Runs natively on `windows-2025`. `scripts/windows-build-local.ps1` builds the
Rust DLL, generates C# UniFFI bindings, publishes the self-contained WPF app,
and packages the NSIS installer and ZIP. The existing Mac-to-Windows SSH wrapper
calls the same entrypoint for local compatibility.

### Linux

Runs `scripts/linux-release` on `ubuntu-24.04`, preserving the existing Docker
build environment and producing the DEB and tarball.

### CLI

One three-entry runner matrix calls `scripts/cli-release` for:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`

Each archive contains `iris/iris`, `iris/install.sh`, and `iris/README.txt`.

### Artifacts

Each build job uploads its own output with `if-no-files-found: error`. The
workflow succeeds only when every retained platform job builds, verifies, and
uploads its required files. GitHub records a digest for each uploaded artifact.

## Simplicity Review

- One top-level workflow owns orchestration.
- Platform build logic stays in scripts instead of YAML.
- One metadata job owns all version decisions.
- YAML anchors reuse immutable checkout and the shared build environment.
- A matrix represents the three structurally identical CLI builds.
- Each platform uploads directly; there is no custom manifest format, artifact
  restaging, final repackaging job, or provenance-only action.
- Existing local release publication remains separate.

## First Run

1. Commit and push the workflow, then merge it to the default branch so manual
   dispatch is available.
2. Confirm `android-release` and `ios-build` contain the settings above.
3. Run **Actions → Build artifacts → Run workflow** with `ref=main`.
4. Approve protected build environments if configured.
5. Download the desired `iris-<platform>-<version>-<sha>` artifact from the
   completed workflow run.

The first hosted run is still required to validate the new native Windows path
and each platform's hosted toolchain.
