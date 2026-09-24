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
- Adaptive voice and video calls over FIPS on Android, iOS, macOS, Windows, Linux and standalone browsers.
- Signed chat events and attachment reads over known native FIPS mesh peers.
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

See [native mesh connections](docs/native-mesh.md) for routing scope and explicit
peer configuration.

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

### Desktop call checks

Windows and Linux use the shared `desktop-media` engine: CPAL audio devices,
Nokhwa cameras, OpenH264 video, the existing Opus codec, and Sonora's WebRTC AEC3
for echo cancellation. Signalling, encryption and media delivery stay on FIPS.
Apple and Android retain their native hardware media paths.

```bash
cargo test --manifest-path core/Cargo.toml --locked --features desktop-media --lib desktop_call -- --nocapture
./scripts/test-linux
# On Windows, after building the native DLL and C# bindings:
./scripts/windows-build-local.ps1 call-tests
```

These checks cover sustained bitrate reduction/recovery, reordered and lost
video, echo cancellation, sample-rate drift, mute/camera privacy, and call UI
lifecycle. Linux needs ALSA development headers and NASM; the dev container
includes them. The Windows build script enables `desktop-media` automatically.

For browser interoperability, build `iris-call-fixture` with
`--features stack-fixture,desktop-media`, then run the browser repository's
`e2e/calls-native.spec.ts` with `IRIS_CALL_FIXTURE_BIN` pointing to that binary
and `IRIS_CALL_DESKTOP_CODEC=1 REQUIRE_CALL_INTEROP=1 CALL_INTEROP_WS_ONLY=1`.
This decodes browser H.264 and re-encodes it with the production desktop codec
over a real local FIPS connection. It uses fresh test accounts and no devices.
Real camera, speakerphone, and Wi-Fi checks still require physical hardware.

`scripts/android_call_lan_e2e.py --help` describes the physical Wi-Fi benchmark.
Build `local_nostr_relay` with `local-relay-bin`, and build paired Android debug
and test APKs with `-Piris.testRunner=to.iris.chat.calls.NativeCallTestRunner`.
Run it inside an Android native-lab reservation with an explicit device and the
laptop's private LAN address. It uses a fresh account, restores the installed
APKs, stops the message server, and checks sustained 150 kbps video, decoded
frame rate, round-trip delay, mute, camera off, and remote hangup. `--voice-only`
also checks an initially muted call lasting longer than the media timeout.

## Build

Native builds require Rust, CMake, and the platform C/C++ toolchain. The bundled
Opus codec builds from source; Android builds use the configured NDK.

```bash
just build
just android-assemble
just ios-xcodeproj
just macos-build
just windows-build
just linux-release
```

Releases are built once in GitHub Actions and promoted unchanged to every
distribution channel. See the [release and distribution playbook](RELEASE.md).

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
- [Android beta release](BETA_RELEASE.md)
- [Architecture](ARCHITECTURE.md)
- [UI/UX flows](UI_UX_FLOWS.md)
- [Windows notes](windows/README.md)
