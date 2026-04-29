# Local Zapstore Release

This project publishes the Android APK to Zapstore from a local machine. Do not
publish from hosted CI for now. The release private keys stay on your own
computer and in your own password manager.

## App Identity

- App name: `Iris Chat`
- Android application ID: `to.iris.chat`
- Repository: `https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs`
- Zapstore channel: `main`
- Listing summary: `Alpha release of Iris Chat, a secure Nostr messenger built on Nostr Double Ratchet.`

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
SIGN_WITH=browser
ZAPSTORE_CHANNEL=main
ZAPSTORE_IDENTITY_RELAYS=wss://relay.zapstore.dev
```

With `SIGN_WITH=browser`, the Nostr private key is not stored in this repo.
Zapstore opens a local browser signing flow instead. Keep the Nostr publisher
key backed up separately so you can import it into a browser signer on a new
computer.

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
Zapstore publisher npub: npub1...
Zapstore publisher nsec: nsec1...
Signing mode: browser
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
cd /Users/l/Projects/iris-fork/iris-chat-rs-cross-platform
cp .env.zapstore.example .env.zapstore.local
chmod 600 .env.zapstore.local
```

Make sure `release.env` exists and points at the local keystore:

```bash
test -f release.env
test -f .zapstore/keystore/iris-chat-release.jks
chmod 600 release.env .zapstore/keystore/iris-chat-release.jks
```

Add your Zapstore publisher `npub` to [zapstore.yaml](/Users/l/Projects/iris-fork/iris-chat-rs-cross-platform/zapstore.yaml:1) before the first real publish:

```yaml
pubkey: npub1...
```

Use the same key in your browser signer.

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

6. Import the Zapstore publisher key into your browser signer.

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
cd /Users/l/Projects/iris-fork/iris-chat-rs-cross-platform
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

The browser signing prompt should appear during `link-identity` and `wizard`.
Confirm that it is signing with the same `npub` listed in `zapstore.yaml`.

## Routine Release Checklist

1. Update version values in `release.env`:

```text
IRIS_APP_VERSION_NAME=0.1.1
IRIS_APP_VERSION_CODE=2
```

2. Update `ZAPSTORE_RELEASE_NOTES.md` with the public notes you want shown in
   Zapstore.

3. Run the normal release/test gate you want for this build.

4. Build and validate the Zapstore APK:

```bash
./scripts/publish-zapstore-android.sh check
```

5. Publish:

```bash
./scripts/publish-zapstore-android.sh publish
```

6. Confirm the new release appears in Zapstore.

7. Update your password manager if any local secret changed.

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
