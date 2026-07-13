# Release Guide

This repo has repeatable release entrypoints for every platform:

- All-in-one local release + hashtree publish: `./scripts/release` (`--publish` to push)
- Android: `./scripts/android-release`
- iOS: `./scripts/ios-release`
- macOS: `./scripts/macos-build macos-dmg` and `./scripts/macos-build macos-updater-archive`
- Windows (x86_64 NSIS installer): `./scripts/windows-build windows-installer`
- Linux (x86_64 tarball + .deb via Docker): `./scripts/linux-release`
- Shared release inputs: copy `release.env.example` to `release.env`

`./scripts/release` builds whatever the current host can build, stages a
hashtree release tree under `dist/release/<tag>/`, and with `--publish` runs
`htree add` + `htree release publish releases/iris-chat-rs <tag> <cid>`, which
also repoints the mutable `latest` release. The desktop updaters use the
bundled `iris` helper to resolve the signed hashtree latest release before
selecting the macOS `.app.tar.gz` or Windows setup `.exe` asset. A publish
with the full CLI archive set also updates the Homebrew tap at
`homebrew-iris.git`, unless
`--skip-homebrew-tap` is passed. Partial builds with `--only` or `--skip` are
for local/staged artifacts and are rejected by `--publish` when they exclude a
required latest platform. By default, latest requires macOS, Windows, Android,
Linux, and CLI artifacts; override with
`IRIS_RELEASE_REQUIRED_LATEST_STEPS` only when the official platform set
changes. `--allow-partial-latest` exists only for an intentional emergency
override. Pass `--dry-run` to see the plan first.

## Official References

- Apple upload/TestFlight workflow:
  - [Upload builds](https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds/)
  - [Add a new app](https://developer.apple.com/help/app-store-connect/create-an-app-record/add-a-new-app/)
  - [TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview)
  - [Distributing your app for beta testing and releases](https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases/)
- Google Play release workflow:
  - [Prepare your app for release](https://developer.android.com/studio/publish/preparing)
  - [Sign your app](https://developer.android.com/guide/publishing/app-signing.html)
  - [Publish your app](https://developer.android.com/studio/publish)
  - [Upload your app to the Play Console](https://developer.android.com/studio/publish/upload-bundle)

## Repo Layout

- `core/`: shared Rust core. Mobile build metadata and default relay sets are
  compiled here via `core/build.rs`.
- `android/`: Gradle/Compose shell. Android package metadata, signing config,
  and Rust Android packaging are controlled from
  `android/app/build.gradle.kts`.
- `ios/`: SwiftUI shell. The Xcode project is generated from `ios/project.yml`,
  while version/build values come from Xcode build settings referenced by
  `ios/Info.plist`.
- `scripts/`: release, test, and build entrypoints.

## Shared Build Inputs

These values are the common boundary between Android, iOS, and the Rust core:

- `IRIS_APP_VERSION_NAME`
- `IRIS_APP_VERSION_CODE`
- `IRIS_BUILD_GIT_SHA`
- `IRIS_BUILD_TIMESTAMP_UTC`
- `IRIS_RELEASE_RELAY_SET_ID`
- `IRIS_RELEASE_RELAYS`

`IRIS_APP_VERSION_NAME` accepts an optional 4th `.build` segment
(`YYYY.M.D.B`) so that re-releases on the same calendar day still get a unique
version name on Zapstore and the htree release tree. The auto-derived
`IRIS_APP_VERSION_CODE` is `major*10000 + minor*1000 + patch*100 + build`, which
also keeps Android `versionCode` strictly increasing across same-day rebuilds.
Apple's App Store rejects 4-part `CFBundleShortVersionString` values, so
`scripts/ios-release` and `scripts/macos-build` strip the `.build` segment
before passing `MARKETING_VERSION` to Xcode; uniqueness on Apple comes from
`CFBundleVersion` (= `IRIS_APP_VERSION_CODE`).

If `IRIS_BUILD_GIT_SHA` and `IRIS_BUILD_TIMESTAMP_UTC` are unset, the release
scripts derive them from the current Git `HEAD`. For stricter reproducibility,
set them explicitly or provide `SOURCE_DATE_EPOCH`.

## Recommended Release Gates

Minimum blocking gate before cutting a release artifact:

```bash
cd /path/to/iris-chat-rs-cross-platform
just qa-native-contract
```

Heavier confidence lane before widening rollout:

```bash
cd /path/to/iris-chat-rs-cross-platform
just qa-interop
```

The full release gate also launches each available app shell with an isolated,
logged-in account containing one direct chat and one group chat. After a
30-second settle period, average CPU over 60 seconds must stay at or below 5%
of one core:

```bash
./scripts/test-release-gate --full
```

Use `./scripts/test-release-gate --idle-cpu` to run just the available platform
CPU lanes plus the normal release prerequisites. macOS and iOS run locally on
macOS, Android uses an online emulator unless `IRIS_ANDROID_SERIAL` explicitly
authorizes another device, Linux uses Docker on macOS, and Windows uses the
configured Windows build host. JSON evidence is written below
`artifacts/idle-cpu/`. Override the limit and timings with
`IRIS_CHAT_IDLE_CPU_MAX_PERCENT`, `IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS`, and
`IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS`.

These scripts do not publish anything. They only verify the build and behavior
surface before packaging.

## Android Organization

Android release inputs are read in this order:

1. `android/local.properties`
2. environment variables

Supported keys:

- App version:
  - `app.versionName` or `IRIS_APP_VERSION_NAME`
  - `app.versionCode` or `IRIS_APP_VERSION_CODE`
- Build metadata:
  - `build.gitSha` or `IRIS_BUILD_GIT_SHA`
  - `build.timestampUtc` or `IRIS_BUILD_TIMESTAMP_UTC`
- Relay/channel config:
  - `debug.relaySetId` or `IRIS_DEBUG_RELAY_SET_ID`
  - `debug.relays` or `IRIS_DEBUG_RELAYS`
  - `beta.relaySetId` or `IRIS_BETA_RELAY_SET_ID`
  - `beta.relays` or `IRIS_BETA_RELAYS`
  - `release.relaySetId` or `IRIS_RELEASE_RELAY_SET_ID`
  - `release.relays` or `IRIS_RELEASE_RELAYS`
- Signing:
  - `beta.storeFile` or `IRIS_BETA_KEYSTORE_PATH`
  - `beta.storePassword` or `IRIS_BETA_KEYSTORE_PASSWORD`
  - `beta.keyAlias` or `IRIS_BETA_KEY_ALIAS`
  - `beta.keyPassword` or `IRIS_BETA_KEY_PASSWORD`
  - `release.storeFile` or `IRIS_RELEASE_KEYSTORE_PATH`
  - `release.storePassword` or `IRIS_RELEASE_KEYSTORE_PASSWORD`
  - `release.keyAlias` or `IRIS_RELEASE_KEY_ALIAS`
  - `release.keyPassword` or `IRIS_RELEASE_KEY_PASSWORD`

Primary commands:

- `./scripts/android-release print-config`
- `./scripts/android-release beta-apk`
- `./scripts/android-release beta-bundle`
- `./scripts/android-release release-apk`
- `./scripts/android-release release-bundle`

Artifacts are copied into `dist/android/` with a stable
`IrisChat-<channel>-<version>+<build>-<sha>` naming scheme, plus a rolling
`IrisChat-<channel>-latest.*` alias and matching `.env` manifests.

## iOS Organization

iOS has two layers:

- `./scripts/ios-build`: native build primitives
  - generate Swift bindings
  - build Rust static libs and XCFramework
  - generate the Xcode project
  - run simulator builds and tests
- `./scripts/ios-release`: release orchestration
  - `print-config`
  - `prepare`
  - `archive`
  - `export`
  - `upload`

iOS release environment:

- `IRIS_IOS_BUNDLE_ID`
- `IRIS_IOS_DEVELOPMENT_TEAM`
- `IRIS_IOS_SIGNING_STYLE`
- `IRIS_IOS_EXPORT_METHOD`
- `IRIS_IOS_INTERNAL_ONLY`
- `IRIS_IOS_ALLOW_PROVISIONING_UPDATES`
- `IRIS_ASC_AUTH_KEY_PATH`
- `IRIS_ASC_AUTH_KEY_ID`
- `IRIS_ASC_AUTH_KEY_ISSUER_ID`

Current defaults:

- bundle ID: `fi.siriusbusiness.irischat`
- signing style: `automatic`
- export method: `app-store-connect`

`ios-release` currently automates automatic signing only.

The generated project takes its version/build from `MARKETING_VERSION` and
`CURRENT_PROJECT_VERSION`, so `ios-release` can archive the same source tree
with explicit release values instead of rewriting plist files in place.

## Step By Step

### Command line Homebrew tap

The release script publishes the CLI formula from the staged release assets:

```bash
brew tap sirius/iris https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/homebrew-iris.git
brew install iris
```

The tap requires these release files:

- `iris-aarch64-apple-darwin.tar.gz`
- `iris-x86_64-apple-darwin.tar.gz`
- `iris-x86_64-unknown-linux-gnu.tar.gz`

Override the tap destination with `IRIS_HOMEBREW_TAP_REPO`,
`IRIS_HOMEBREW_TAP_NAME`, or `IRIS_HOMEBREW_TAP_PUSH_URL`. If intentionally
using a shared tap, seed from the existing tap so other formulas are preserved.

### Android closed test or release

1. Copy `release.env.example` to `release.env`.
2. Fill `IRIS_APP_VERSION_NAME`, `IRIS_APP_VERSION_CODE`, relay values, and
   signing values.
3. Inspect the resolved config:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/android-release print-config
```

4. Build the target artifact:

```bash
./scripts/android-release release-bundle
```

5. Upload the `.aab` from `dist/android/` to the correct Play track.
6. For a side-loadable trusted beta, use `./scripts/android-release beta-apk`
   or `./scripts/android-release beta-bundle`.

### iOS TestFlight

1. In App Store Connect, create the app record first.
2. Copy `release.env.example` to `release.env`.
3. Fill `IRIS_APP_VERSION_NAME`, `IRIS_APP_VERSION_CODE`, relay values,
   `IRIS_IOS_BUNDLE_ID`, and `IRIS_IOS_DEVELOPMENT_TEAM`.
4. If you want Xcode to create/fetch signing assets, set
   `IRIS_IOS_ALLOW_PROVISIONING_UPDATES=true`.
5. Inspect the resolved config:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/ios-release print-config
```

6. Build the archive:

```bash
./scripts/ios-release archive
```

7. Export an IPA if you want a local artifact:

```bash
./scripts/ios-release export
```

8. Upload either from Xcode Organizer or with:

```bash
./scripts/ios-release upload
```

9. Wait for App Store Connect processing, then add the build to internal or
   external TestFlight groups.

## Reproducibility Notes

The release scripts normalize:

- version/build
- git SHA
- build timestamp
- release relay configuration
- artifact naming
- per-artifact `.env` manifests in `dist/`

These scripts improve repeatability, but they do not guarantee bit-for-bit
identical output across different toolchain versions or machines. To tighten
that further, keep Xcode, Gradle, Android SDK/NDK, JDK, and Cargo inputs pinned
and build from a clean commit with explicit metadata.

## Current Limits

- `ios-release` automates automatic signing only. Manual provisioning-profile
  mapping is not encoded in the script.
- App Store Connect metadata, tester groups, screenshots, privacy
  questionnaires, and review submissions still happen in Apple/Google consoles.
- `qa-interop` is intentionally a heavier confidence lane, not a per-commit
  blocking gate.
