# Parity Matrix

Status tracking for the shared Iris Chat workspace.

Legend:

- `Done`: implemented and verified locally
- `Partial`: implemented but not fully acceptance-tested yet
- `Planned`: not implemented yet

| Workflow | Rust | Android | iOS | Acceptance |
| --- | --- | --- | --- | --- |
| Create owner account | Done | Done | Done | Done |
| Restore from owner `nsec` | Done | Done | Done | Partial |
| Start linked device from owner QR/paste | Done | Done | Done | Partial |
| Approve pending linked device | Done | Done | Done | Partial |
| Remove authorized device | Done | Done | Done | Partial |
| Device revoked screen | Done | Done | Done | Partial |
| Chat list routing | Done | Done | Done | Done |
| Create direct chat | Done | Done | Done | Done |
| Send direct message | Done | Done | Done | Done |
| Create group | Done | Done | Done | Done |
| Rename group | Done | Done | Done | Partial |
| Add group members | Done | Done | Done | Partial |
| Remove group members | Done | Done | Done | Partial |
| Group details screen | Done | Done | Done | Done |
| Profile sheet | Done | Done | Done | Done |
| Owner QR display | Done | Done | Done | Done |
| Support bundle export/copy | Done | Done | Done | Partial |
| Shared device-approval QR codec | Done | Done | Done | Done |
| Android run tooling from repo root | n/a | Done | n/a | Done |
| iOS run tooling from repo root | n/a | n/a | Done | Done |
| Root repo self-contained build | Done | Done | Done | Done |
| Android AppManager contract tests | n/a | Done | n/a | Done |
| Android secure-store tests | n/a | Done | n/a | Done |
| iOS Keychain store tests | n/a | n/a | Done | Done |
| iOS AppManager reconcile tests | n/a | n/a | Done | Done |
| Blocking native-contract gate (`just qa-native-contract`) | Done | Done | Done | Done |
| Android/iOS interop smoke matrix | Done | Done | Done | Done |
| iOS <-> iOS chat acceptance | Done | n/a | Partial | Planned |
| iOS <-> Android chat acceptance | Done | Done | Done | Done |
| Restore history convergence on iOS | Done | n/a | Partial | Planned |

## Current Notes

- The repo is structured as `core/`, `android/`, and `ios/`, with protocol
  runtime code consumed as an external Rust dependency.
- Android and iOS both consume the same UniFFI surface from `core/`.
- The device approval QR format is owned by `core/` and generated into both
  native clients.
- `just qa-native-contract` is green locally and is the blocking gate before
  Rust-core refactors.
- `just qa-interop` is the heavier, non-blocking confidence lane for mixed
  relay-backed flows.
- The mixed Android+iOS matrix currently succeeds locally for direct-chat
  transport and mixed group messaging in both creator directions.
