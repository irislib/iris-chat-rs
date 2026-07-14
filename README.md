# iris chat

Encrypted chat app using Nostr Double Ratchet. Shared Rust core, native UIs.

Primary development is on hashtree:
https://git.iris.to/#/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/iris-chat-rs

## Features

- Encrypted direct and group chats.
- Device linking and QR/link invites.
- Offline queueing, message server sync, and SQLite persistence.
- Reliable ordered linked-device snapshots over TCP/FIPS, while Delivered and
  Seen remain recipient application receipts.
- `iris` command line app for scripts, agents, and local devices that need to
  send messages.
- Attachments, profile pictures, notifications, and support bundles.
- Nearby chat over Wi-Fi/LAN and Bluetooth.
- Desktop open-at-login on macOS, Linux, and Windows.
- Share to iris chat from Android, iOS, and macOS.
- Search and choose one or more recipients.
- iOS suggests recent chats in the share sheet.

## Status

- Shared Rust core drives app state, navigation, messaging, sync, persistence,
  and support export across platforms.
- Native shells exist for Android, iOS, macOS, Linux, and Windows.
- Android, iOS, and macOS are the most active app targets.
- Linux and Windows are buildable and releaseable, with lighter acceptance
  coverage.

## Repo

- `core/`: Rust core and UniFFI boundary
- `android/`: Android Compose app
- `ios/`: iOS SwiftUI app and shared Apple shell code
- `macos/`: macOS SwiftUI app
- `linux/`: GTK/libadwaita desktop app
- `windows/`: WPF/.NET desktop app
- `scripts/`: build, test, release, and harness entrypoints
- `docs/`: feature and release docs

## Run

```bash
cd /path/to/iris-chat-rs
just info
just run
just build
just run-android
just run-ios
just run-linux
just run-macos
just run-windows
```

`just run` dispatches to the native app for the local desktop platform.
`just build` builds the native app for the local desktop platform and prints
the app output path.

## Check

```bash
just verify-fast
just verify-health
just verify-full
just qa-native-contract
just qa-interop
just qa-lan
```

`verify-fast` is the per-change Rust/core/contract tier and does not allocate
simulators, phones, VMs, or GUI sessions. `verify-full` reserves the native lab,
runs the five-platform plus physical-device matrices, and is intended for
nightly or release boundaries. Machine-readable results distinguish
`infrastructure_unavailable` (exit 75) from product failures. See
`docs/verification-tiers.md` for resource configuration and safe reset rules.

## Build

```bash
just build
just android-assemble
just ios-xcodeproj
just macos-build
just windows-build
just linux-release
```

Release helpers:

```bash
just release
just release-publish
./scripts/android-release
./scripts/ios-release
just macos-dmg
just windows-installer
just linux-release
```

`just release-publish` stages release artifacts under `dist/release/`
and publishes the release tree to hashtree. It runs the release gate first,
publishes a new `iris-chat` crate version when needed, and sends iOS builds to
internal and public TestFlight unless skipped.

## Command Line

Install the Iris command line app with Homebrew:

```bash
brew tap sirius/iris https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/homebrew-iris.git
brew install iris
```

Or install a prebuilt macOS/Linux binary directly:

```bash
curl -fsSL https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/releases%2Firis-chat-rs/latest/install.sh | sh
```

Or build it from crates.io with Cargo:

```bash
cargo install iris-chat
iris --help
```

The `iris` command is useful for humans, agents, scripts, and local devices
that need to send, search, or listen for messages and trigger normal iris chat
notifications.

Messages can travel over Nostr relays, and nearby transports can keep local
device messages off a remote server when the devices are close enough.

## Platform Notes

- Android: Compose UI, Gradle, Rust via `cargo-ndk`, Zapstore release path.
- iOS: SwiftUI, XcodeGen, share extension, push support, App Store archive
  helper.
- macOS: SwiftUI, XcodeGen, share extension, LaunchAgent open-at-login, DMG
  helper.
- Linux: GTK/libadwaita shell, direct Rust core link, XDG open-at-login.
- Windows: WPF/.NET 8 shell, x86_64 MSVC target, Credential Manager,
  open-at-login via the Run key.

## More

- [Release guide](RELEASE.md)
- [Zapstore release](docs/release-zapstore.md)
- [Android beta release](BETA_RELEASE.md)
- [Architecture](ARCHITECTURE.md)
- [UI/UX flows](UI_UX_FLOWS.md)
- [Windows notes](windows/README.md)
