# Android Beta Release

This repo has a dedicated Android `beta` build intended for a trusted closed
test group.

## Beta Warning

- The beta is for trusted testers only.
- Local app-core state is not encrypted at rest yet.
- Do not use the beta for sensitive conversations.

The trusted-test warning is surfaced in both Android welcome/profile UI and the
iOS profile UI when the build is configured as a trusted test build.

## Build Configuration

The `beta` build type is release-like, optimized, and installable alongside
`debug`.

Build metadata is embedded in the app UI and support bundle:

- app version
- git SHA
- build timestamp
- relay-set ID
- build channel

## Relay Configuration

The Rust core reads its compiled relay defaults from the Android build.

Override values before packaging the beta:

- `beta.relaySetId` or `IRIS_BETA_RELAY_SET_ID`
- `beta.relays` or `IRIS_BETA_RELAYS`

Optional dedicated beta signing config:

- `beta.storeFile` or `IRIS_BETA_KEYSTORE_PATH`
- `beta.storePassword` or `IRIS_BETA_KEYSTORE_PASSWORD`
- `beta.keyAlias` or `IRIS_BETA_KEY_ALIAS`
- `beta.keyPassword` or `IRIS_BETA_KEY_PASSWORD`

If dedicated beta signing is not supplied, the beta build falls back to release
signing and then to debug signing.

## Support Flow

Open the profile sheet to:

- share a redacted support bundle
- copy the support bundle JSON
- reset app state

The support bundle intentionally excludes:

- secret keys
- session secrets
- raw persisted protocol snapshots
- message bodies

## Recommended Gates

Fast local gate:

```bash
cd /path/to/iris-chat-rs-cross-platform
just qa
```

Blocking native-shell gate before cutting a beta:

```bash
cd /path/to/iris-chat-rs-cross-platform
just qa-native-contract
./scripts/test_beta_local.sh
```

Heavier confidence lane before widening the tester group:

```bash
cd /path/to/iris-chat-rs-cross-platform
just qa-interop
./scripts/local_relay_scenario_soak.sh --iterations 100
./scripts/group_chat_matrix_smoke.sh
```

`./scripts/test_beta_local.sh` keeps the Android build surface honest for:

- Rust tests
- Android debug compile
- Android beta compile
- Android androidTest compile

## Packaging

Print the resolved release inputs:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/android-release print-config
```

Build the beta APK:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/android-release beta-apk
```

Output:

- `dist/android/IrisChat-beta-<version>+<build>-<sha>.apk`
- matching `dist/android/IrisChat-beta-<version>+<build>-<sha>.env`

Build the Play-ready beta bundle:

```bash
cd /path/to/iris-chat-rs-cross-platform
./scripts/android-release beta-bundle
```

Output:

- `dist/android/IrisChat-beta-<version>+<build>-<sha>.aab`
- matching `dist/android/IrisChat-beta-<version>+<build>-<sha>.env`

## Manual Acceptance

Before inviting testers, verify on at least one real phone:

1. Install the beta APK.
2. Confirm the trusted-test warning is visible.
3. Create an account.
4. Link a second device.
5. Send and receive a direct message.
6. Create a group.
7. Send and receive group messages.
8. Remove a member and confirm future sends are blocked.
9. Force-stop and reopen the app, then verify chats/groups restore.
10. Export and share a support bundle.

## Upgrade Smoke

Before publishing beta `N+1`:

1. Install beta `N`.
2. Create direct chats and a group.
3. Install beta `N+1` over beta `N`.
4. Reopen the app and verify restore.
5. Send and receive both direct and group messages again.
