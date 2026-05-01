# iris chat

Encrypted chat app using Nostr Double Ratchet. Shared Rust core, native UIs.

Primary development is on hashtree:
https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs

The compatible `ndr` command line app lives in `nostr-double-ratchet`:
https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-double-ratchet

## Features

- Encrypted direct and group chats.
- Device linking and QR/link invites.
- Offline queueing, message server sync, and SQLite persistence.
- Compatible with the `ndr` command line app for scripts, agents, and local
  devices that need to send messages.
- Attachments, profile pictures, notifications, and support bundles.
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
just run-android
just run-ios
just run-macos
just run-windows
```

Linux is covered by `just test-linux` and `just linux-release`; it does not
currently have a `just run-linux` shortcut.

## Check

```bash
just qa
just test-all-platforms
just qa-native-contract
just qa-interop
```

Use `just qa` for the normal local gate. Use the interop lanes before core or
sync changes.

## Build

```bash
just android-assemble
just ios-xcodeproj
just macos-build
just windows-build
just linux-release
```

Release helpers:

```bash
./scripts/release --publish
./scripts/android-release
./scripts/ios-release
./scripts/macos-build macos-dmg
./scripts/windows-build windows-installer
./scripts/linux-release
```

`./scripts/release --publish` stages release artifacts under `dist/release/`
and publishes the release tree to hashtree.

## Command Line

Install the Iris command line app with Cargo:

```bash
cargo install iris-chat
iris --help
```

The `iris` command is useful for agents, programs, scripts, and local devices
that need to send or search messages and trigger normal iris chat notifications.

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
- [Parity matrix](PARITY_MATRIX.md)
- [Architecture](ARCHITECTURE.md)
- [UI/UX flows](UI_UX_FLOWS.md)
- [Windows notes](windows/README.md)
