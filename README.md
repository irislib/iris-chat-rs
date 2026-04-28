# Iris Chat

Iris Chat is a Rust-first mobile workspace built on Nostr Double Ratchet.

`core/` owns the app model, router, messaging logic, relay/runtime behavior,
and persistence. Android and iOS are thin native shells that restore secure
credentials, persist secure side effects, render `AppState`, and forward
`AppAction` back to Rust.

## Repo Shape

- `core/`: shared Rust app core and UniFFI boundary
- `android/`: Android shell, Gradle project, and Compose UI
- `ios/`: iOS shell, SwiftUI UI, XcodeGen spec, and tests
- `macos/`: macOS shell, SwiftUI UI, and XcodeGen spec reusing the Apple shell layer
- `scripts/`: build, test, release, emulator, simulator, and harness entrypoints
- `tools/`: higher-signal local run/doctor wrappers

## Docs

- [Architecture](ARCHITECTURE.md)
- [Architecture review](ARCHITECTURE_REVIEW.md)
- [Implementation plan](ARCHITECTURE_IMPLEMENTATION_PLAN.md)
- [UI/UX flows](UI_UX_FLOWS.md)
- [Parity matrix](PARITY_MATRIX.md)
- [Release guide](RELEASE.md)
- [Android beta release](BETA_RELEASE.md)
- [Zapstore release](docs/release-zapstore.md)

## Get Started

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/mobile_bootstrap_macos.sh
just info
just run-android
just run-ios
just run-macos
```

## Daily Test Lanes

- `just qa`
  - Rust tests
  - one local-relay soak iteration
  - Android debug compile gates
  - iOS XCTest and UI tests
- Platform app entrypoints:
  - `just test-android`
  - `just test-ios`
  - `just test-macos`
  - `just test-linux`
  - `just test-windows`
- `just test-all-platforms`
  - Rust core tests
  - every platform app test entrypoint above
- `just qa-native-contract`
  - `just qa`
  - Android `AppManagerContractTest`
  - Android `PikaLikeUiTest`
  - Android `AndroidKeystoreSecretStoreTest`
- `just qa-interop`
  - mixed Android+iOS group/direct matrix
  - Android restore/group relay smoke
  - Android linked-device relay matrix

Use `just qa-native-contract` as the blocking gate before refactoring the Rust
core. Use `just qa-interop` as the heavier confidence lane.

## Android

Build and install the debug app:

```bash
cd /path/to/iris-chat-rs-cross-platform
just android-assemble
./scripts/emulator_smoke.sh --clear emulator-5554 emulator-5556 emulator-5558
```

The Android emulator launcher passes explicit DNS servers by default because
relay publishing depends on `wss://` hostname resolution. Override the default
with `IRIS_ANDROID_DNS_SERVERS=1.1.1.1,8.8.8.8 just run-android`, or set
`IRIS_ANDROID_DNS_SERVERS=off` to use the emulator's inherited resolver setup.

Build release artifacts:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/android-release print-config
./scripts/android-release beta-apk
./scripts/android-release beta-bundle
./scripts/android-release release-bundle
```

Android release details live in [BETA_RELEASE.md](BETA_RELEASE.md) and
[RELEASE.md](RELEASE.md).

## iOS

The iOS app is generated from `ios/project.yml` and links the shared Rust core
through generated Swift bindings plus `ios/Frameworks/IrisChatCore.xcframework`.

Common local flows:

```bash
cd /path/to/iris-chat-rs-cross-platform
just ios-gen-swift
just ios-xcframework
just ios-xcodeproj
just run-ios
./scripts/ios-build ios-test
```

Prepare and archive a release build:

```bash
cd /path/to/iris-chat-rs-cross-platform
cp release.env.example release.env
$EDITOR release.env
./scripts/ios-release print-config
./scripts/ios-release archive
```

## macOS

The first native desktop shell is a macOS SwiftUI target generated from
`macos/project.yml`. It reuses the shared Apple shell layer in `ios/Sources/`
and links the Rust core through `macos/Frameworks/IrisChatCore.xcframework`.

Common local flows:

```bash
cd /path/to/iris-chat-rs-cross-platform
just macos-gen-swift
just macos-xcframework
just macos-xcodeproj
just run-macos
```

## Interop Harnesses

Repo-native harnesses exist for the heavy relay-backed matrixes:

- Android harness runner: `scripts/run_harness.py`
- iOS harness runner: `scripts/run_ios_harness.py`
- mixed-platform matrix: `scripts/mixed_platform_group_chat_matrix.sh`

These are intentionally separate from the fast local UI smoke suites.
