# Local Zapstore Release

This project publishes the Android APK to Zapstore from a local machine. Do not
publish from hosted CI for now. The release private keys stay on your own
computer and in your own password manager.

The preferred current flow is split:

- GitHub Actions builds and signs the Android APK.
- A local machine downloads that signed APK artifact and publishes it to
  Zapstore with a local Zapstore/Nostr signing key.

That keeps Android signing in CI while keeping Zapstore publishing local.

## App Identity

- App name: `Iris Chat`
- Android application ID: `to.iris.chat`
- Repository: `https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs`
- Zapstore channel: `main`
- Zapstore app metadata publisher: `npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm`
- Automated release signer: `npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm`
- Listing summary: `End-to-end encrypted chat app using Nostr Double Ratchet.`

Treat the Android application ID, Android signing key, and Zapstore publisher
Nostr identity as long-lived release identity. Changing them later can make
updates harder or break trust continuity.

## Local Secret Files

These files are intentionally ignored by git:

- `release.env`
- `.env.zapstore.local`
- `.zapstore/`

Current local keystore layout:

```text
.zapstore/keystore/iris-chat-release.jks
```

Current Android key alias:

```text
iris-chat-upload
```

`release.env` contains Android release signing values:

```text
IRIS_RELEASE_KEYSTORE_PATH
IRIS_RELEASE_KEYSTORE_PASSWORD
IRIS_RELEASE_KEY_ALIAS
IRIS_RELEASE_KEY_PASSWORD
```

`.env.zapstore.local` contains Zapstore publish settings:

```text
NOSTR_KEY_PATH=/absolute/path/to/nsec
ZAPSTORE_CHANNEL=main
ZAPSTORE_IDENTITY_RELAYS=wss://relay.zapstore.dev
```

`SIGN_WITH=nsec1...` also works, but `NOSTR_KEY_PATH` is preferred so the key
does not appear in command history or process listings. If neither value is
set, the script falls back to `SIGN_WITH=browser`.

Recommended local Nostr key path:

```text
~/.config/iris-chat/zapstore-nsec
```

Create it without putting the `nsec` in shell history:

```bash
mkdir -p ~/.config/iris-chat
chmod 700 ~/.config/iris-chat
read -rsp "Zapstore nsec: " NSEC; echo
printf '%s' "$NSEC" > ~/.config/iris-chat/zapstore-nsec
unset NSEC
chmod 600 ~/.config/iris-chat/zapstore-nsec
```

Then point `.env.zapstore.local` at it:

```bash
cat > .env.zapstore.local <<EOF
NOSTR_KEY_PATH=$HOME/.config/iris-chat/zapstore-nsec
ZAPSTORE_CHANNEL=main
ZAPSTORE_IDENTITY_RELAYS=wss://relay.zapstore.dev
EOF
chmod 600 .env.zapstore.local
```

If this repo does not have its own `.env.zapstore.local`, the publish script
will use `../nostr-vpn/.env.zapstore.local` when present. Routine releases use
that local key for release-only Zapstore events while leaving the existing app
metadata event owned by the Zapstore app metadata publisher.

## What To Store Permanently

Store these in a password manager such as 1Password, Bitwarden, iCloud
Keychain secure notes, or another encrypted vault you trust:

- The file `.zapstore/keystore/iris-chat-release.jks`.
- The full contents of `release.env`.
- The full contents of `.env.zapstore.local`.
- The Zapstore publisher `npub`.
- The Zapstore publisher `nsec`, unless you intentionally keep it only in a hardware signer or browser signer backup.
- A note that this key is for `Iris Chat / to.iris.chat / Zapstore`.

Do not store the only copy of the Android keystore on one laptop. Losing it
means future APK updates signed with the same key may become impossible.

## Recommended Password Manager Entry

Create one secure item named:

```text
Iris Chat Zapstore Release
```

Suggested fields:

```text
Android app ID: to.iris.chat
Primary repo: https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs
Keystore file: attach iris-chat-release.jks
Keystore alias: iris-chat-upload
Keystore password: <from release.env>
Key password: <from release.env>
Zapstore channel: main
Zapstore app metadata publisher npub: npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm
Automated release signer npub: npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm
Automated release signer nsec: nsec1...
Signing mode: NOSTR_KEY_PATH
Local keystore path: .zapstore/keystore/iris-chat-release.jks
```

If the password manager supports file attachments, attach the `.jks` file. If
it does not, store the `.jks` in an encrypted disk image, encrypted archive, or
another encrypted file vault and record where it lives.

## One-Time Setup On This Computer

Install `zsp`:

```bash
go install github.com/zapstore/zsp@latest
```

Make sure `zsp` is on your path:

```bash
export PATH="$(go env GOPATH)/bin:$PATH"
zsp --help
```

Create local Zapstore settings if they do not exist:

```bash
cd /path/to/iris-chat-rs
cat > .env.zapstore.local <<EOF
NOSTR_KEY_PATH=$HOME/.config/iris-chat/zapstore-nsec
ZAPSTORE_CHANNEL=main
ZAPSTORE_IDENTITY_RELAYS=wss://relay.zapstore.dev
EOF
chmod 600 .env.zapstore.local
```

If you also need to build/sign Android locally, make sure `release.env` exists
and points at the local keystore:

```bash
test -f release.env
test -f .zapstore/keystore/iris-chat-release.jks
chmod 600 release.env .zapstore/keystore/iris-chat-release.jks
```

Add the Zapstore app metadata publisher `npub` to `zapstore.yaml` before the
first real app metadata publish:

```yaml
pubkey: npub1...
```

For routine `scripts/release --publish` runs, the release script publishes
release-only Zapstore events with `--skip-app-event` and the automated release
signer. Use the app metadata publisher key only when intentionally replacing the
kind `32267` app metadata event.

## Restore On A New Computer

1. Clone the repo:

```bash
git clone htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs
cd iris-chat-rs
```

2. Restore the ignored files from your password manager:

```text
release.env
.env.zapstore.local
.zapstore/keystore/iris-chat-release.jks
```

3. Fix local file permissions:

```bash
chmod 600 release.env .env.zapstore.local .zapstore/keystore/iris-chat-release.jks
chmod 700 .zapstore .zapstore/keystore
```

4. Install the normal Android/Rust build prerequisites for this repo.

5. Install `zsp`:

```bash
go install github.com/zapstore/zsp@latest
export PATH="$(go env GOPATH)/bin:$PATH"
```

6. Restore `NOSTR_KEY_PATH` in `.env.zapstore.local`, or import the Zapstore
   publisher key into your browser signer if you use browser signing.

7. Verify the restored setup:

```bash
./scripts/publish-zapstore-android.sh doctor
./scripts/publish-zapstore-android.sh print-config
./scripts/publish-zapstore-android.sh check
```

`doctor` checks the ignored secret files and Android keystore without printing
passwords. `check` builds the signed release APK and validates the Zapstore
config without publishing.

## First Publish Checklist

Run from the repo root:

```bash
cd /path/to/iris-chat-rs
```

Verify config:

```bash
./scripts/publish-zapstore-android.sh doctor
./scripts/publish-zapstore-android.sh print-config
```

Build and validate:

```bash
./scripts/publish-zapstore-android.sh check
```

Link the Android signing certificate to the Zapstore publisher identity:

```bash
./scripts/publish-zapstore-android.sh link-identity
```

Run the interactive first publish:

```bash
./scripts/publish-zapstore-android.sh wizard
```

Confirm that `link-identity` and `wizard` sign with the same `npub` listed in
`zapstore.yaml`. A browser prompt appears only when using browser signing.

## Publish A CI-Signed APK Locally

Run the Android Release APK workflow from GitHub:

```bash
gh workflow run android-release-apk.yml \
  --repo irislib/iris-chat-rs \
  --ref main \
  -f create_tag=true \
  -f artifact_retention_days=30
```

Wait for it to complete:

```bash
gh run list --repo irislib/iris-chat-rs --workflow android-release-apk.yml --limit 1
gh run watch <RUN_ID> --repo irislib/iris-chat-rs --exit-status
```

Publish the APK from that run to Zapstore:

```bash
./scripts/publish-zapstore-github-apk --run-id <RUN_ID>
```

If you omit `--run-id`, the script uses the latest successful
`android-release-apk.yml` run:

```bash
./scripts/publish-zapstore-github-apk
```

Dry validation without publishing:

```bash
./scripts/publish-zapstore-github-apk --run-id <RUN_ID> --check
```

Interactive Zapstore publish:

```bash
./scripts/publish-zapstore-github-apk --run-id <RUN_ID> --wizard
```

The script downloads the GitHub artifact, stages the selected signed APK at
`dist/android/IrisChat-release-latest.apk`, prints CI metadata when present,
then invokes `scripts/publish-zapstore-android.sh` with `ZAPSTORE_APK_PATH` so
the local machine does not need Android signing credentials.

Assumptions:

- `gh` is authenticated with access to `irislib/iris-chat-rs`.
- `zsp` is installed and on `PATH`.
- `zapstore.yaml` contains the intended Zapstore app metadata publisher `npub`.
- `.env.zapstore.local` points `NOSTR_KEY_PATH` at the local `nsec`, or
  `SIGN_WITH` is set another way.
- The automated release signer identity is allowed to publish this package/listing.
  If Zapstore requires identity linking for the new Android signing key, that
  one-time link must be handled before routine publishes.

## Routine Release Checklist

1. Choose the release version. The GitHub workflow can derive a UTC calendar
   version from tags automatically, or you can pass explicit values:

```bash
gh workflow run android-release-apk.yml \
  --repo irislib/iris-chat-rs \
  --ref main \
  -f version_name=2026.6.21 \
  -f version_code=20260621 \
  -f create_tag=true \
  -f artifact_retention_days=30
```

2. Update `ZAPSTORE_RELEASE_NOTES.md` with the public notes you want shown in
   Zapstore.

3. Run the normal release/test gate you want for this build.

4. If you did not start the workflow in step 1, start it now and let it derive
   the version:

```bash
gh workflow run android-release-apk.yml \
  --repo irislib/iris-chat-rs \
  --ref main \
  -f create_tag=true \
  -f artifact_retention_days=30
```

5. Wait for the workflow run:

```bash
gh run list --repo irislib/iris-chat-rs --workflow android-release-apk.yml --limit 1
gh run watch <RUN_ID> --repo irislib/iris-chat-rs --exit-status
```

6. Validate and publish the CI-signed APK locally:

```bash
./scripts/publish-zapstore-github-apk --run-id <RUN_ID> --check
./scripts/publish-zapstore-github-apk --run-id <RUN_ID>
```

To publish only Zapstore from an already-downloaded or locally-built APK:

```bash
./scripts/publish-zapstore-android.sh publish
```

7. Confirm the new release appears in Zapstore.

8. Update your password manager if any local secret changed.

## Useful Verification Commands

Show the APK package identity:

```bash
SDK_DIR="$(sed -n 's/^sdk\.dir=//p' android/local.properties | tail -n 1)"
AAPT="$(find "$SDK_DIR/build-tools" -name aapt -type f | sort | tail -n 1)"
"$AAPT" dump badging dist/android/IrisChat-release-latest.apk | sed -n '1,8p'
```

Expected package:

```text
to.iris.chat
```

Show the Android signing certificate fingerprint:

```bash
set -a
source release.env
set +a

keytool -list -v \
  -keystore .zapstore/keystore/iris-chat-release.jks \
  -storepass "$IRIS_RELEASE_KEYSTORE_PASSWORD"
```

Check local secret wiring without printing passwords:

```bash
./scripts/publish-zapstore-android.sh doctor
```

Validate Zapstore config without publishing:

```bash
./scripts/publish-zapstore-android.sh check
```

## Recovery Notes

If you lose `release.env` but still have the keystore and passwords in your
password manager, recreate `release.env` from `release.env.example` and fill in
the same keystore values.

If you lose the keystore, do not generate a replacement and publish without
thinking through the consequences. A replacement key changes the APK signing
identity. Check Zapstore update and identity-linking behavior before publishing
with a new key.

If you lose the Zapstore publisher Nostr key, do not publish from a new key
until you understand the trust and listing consequences. The publisher identity
is part of the app trust chain.
