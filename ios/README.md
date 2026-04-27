# iOS

This directory contains the native iOS shell for Iris Chat.

## What Lives Here

- `Sources/`
  - SwiftUI app shell and platform adapters
  - `AppManager.swift` is the native bridge over the Rust core
- `Tests/`
  - unit and shell-contract coverage
  - includes the checked-in interop harness test target used by
    `scripts/run_ios_harness.py`
- `UITests/`
  - local iOS smoke tests
- `Bindings/`
  - UniFFI-generated Swift bindings
- `Frameworks/IrisChatCore.xcframework`
  - packaged Rust static library output
- `project.yml`
  - XcodeGen source of truth for the generated project

## Build Flow

Do not edit the generated Xcode project manually. Regenerate it from
`project.yml` through the repo scripts.

Common commands:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/ios-build ios-gen-swift
./scripts/ios-build ios-xcframework
./scripts/ios-build ios-xcodeproj
./scripts/ios-build ios-test
```

## Release Flow

Release orchestration happens through `../scripts/ios-release`:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/ios-release print-config
./scripts/ios-release archive
./scripts/ios-release export
./scripts/ios-release upload
```

The default bundle ID is `to.iris.chat` (matches the Flutter app, dev team `J8PPJKD7TA`).

## Harness Flow

Heavy mixed-platform and relay-backed checks use the checked-in iOS harness:

- runner: `../scripts/run_ios_harness.py`
- test target: `Tests/InteropHarnessTests.swift`

This is separate from the local UI smoke suite on purpose.

## Shell Responsibility

The iOS shell is intentionally thin. It should:

- construct the Rust core
- restore secure credentials into Rust
- persist secure side effects from Rust
- render Rust `AppState`
- forward user actions and platform events back to Rust

Business logic, navigation state, and authoritative app truth stay in `../core`.
