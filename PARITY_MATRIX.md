# Parity Matrix

Status tracking for the shared Iris Chat workspace.

Legend:

- `Done`: implemented and verified locally
- `Partial`: implemented but not fully acceptance-tested yet
- `Planned`: not implemented yet

| Workflow | Rust | Android | iOS | Linux | Acceptance |
| --- | --- | --- | --- | --- | --- |
| Create owner account | Done | Done | Done | Done | Done |
| Restore from owner `nsec` | Done | Done | Done | Done | Partial |
| Start linked device from owner QR/paste | Done | Done | Done | Partial | Partial |
| Approve pending linked device | Done | Done | Done | Planned | Partial |
| Remove authorized device | Done | Done | Done | Planned | Partial |
| Device revoked screen | Done | Done | Done | Planned | Partial |
| Chat list routing | Done | Done | Done | Partial | Done |
| Create direct chat | Done | Done | Done | Planned | Done |
| Send direct message | Done | Done | Done | Planned | Done |
| Create group | Done | Done | Done | Planned | Done |
| Rename group | Done | Done | Done | Planned | Partial |
| Add group members | Done | Done | Done | Planned | Partial |
| Remove group members | Done | Done | Done | Planned | Partial |
| Group details screen | Done | Done | Done | Planned | Done |
| Profile sheet | Done | Done | Done | Planned | Done |
| Owner QR display | Done | Done | Done | Planned | Done |
| Support bundle export/copy | Done | Done | Done | Planned | Partial |
| Shared device-approval QR codec | Done | Done | Done | n/a | Done |
| Android run tooling from repo root | n/a | Done | n/a | n/a | Done |
| iOS run tooling from repo root | n/a | n/a | Done | n/a | Done |
| Linux run tooling from repo root | n/a | n/a | n/a | Done | Done |
| Root repo self-contained build | Done | Done | Done | Done | Done |
| Android AppManager contract tests | n/a | Done | n/a | n/a | Done |
| Android secure-store tests | n/a | Done | n/a | n/a | Done |
| iOS Keychain store tests | n/a | n/a | Done | n/a | Done |
| iOS AppManager reconcile tests | n/a | n/a | Done | n/a | Done |
| Linux secret-store + AppManager reconcile tests | n/a | n/a | n/a | Planned | Planned |
| Blocking native-contract gate (`just qa-native-contract`) | Done | Done | Done | Planned | Done |
| Android/iOS interop smoke matrix | Done | Done | Done | n/a | Done |
| iOS <-> iOS chat acceptance | Done | n/a | Partial | n/a | Planned |
| iOS <-> Android chat acceptance | Done | Done | Done | n/a | Done |
| Restore history convergence on iOS | Done | n/a | Partial | n/a | Planned |

## Current Notes

- The repo is structured as `core/`, `android/`, `ios/`, and `linux/`, with
  protocol runtime code consumed as an external Rust dependency.
- Android and iOS consume the UniFFI surface from `core/`. The Linux GTK4 +
  libadwaita shell consumes `core/` directly as an rlib (no UniFFI) and uses
  a file-backed secret store as a placeholder for libsecret/oo7.
- The device approval QR format is owned by `core/` and generated into both
  native clients.
- `just qa-native-contract` is green locally and is the blocking gate before
  Rust-core refactors.
- `just qa-interop` is the heavier, non-blocking confidence lane for mixed
  relay-backed flows.
- The mixed Android+iOS matrix currently succeeds locally for direct-chat
  transport and mixed group messaging in both creator directions.
