# Homebrew Packaging

The Iris CLI tap is published as a plain Git repository over the hashtree
gateway. Homebrew can tap it directly from the static HTTP URL:

```bash
brew tap sirius/iris https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/homebrew-iris.git
brew install iris
```

The formula installs the `iris` command from the release CLI archives:

- `iris-aarch64-apple-darwin.tar.gz`
- `iris-x86_64-apple-darwin.tar.gz`
- `iris-x86_64-unknown-linux-gnu.tar.gz`

Linux ARM Homebrew installs are intentionally not enabled until the release
builder produces a matching `iris-aarch64-unknown-linux-gnu.tar.gz` archive.

## Publish

`./scripts/release --publish` updates the tap after the htree release has been
published. To run the tap step manually:

```bash
packaging/homebrew/publish_tap.sh \
  --version v<version> \
  --release-base-url https://upload.iris.to/<npub>/releases%2Firis-chat-rs/v<version>/assets \
  --assets-dir dist/release/v<version>/assets
```

Defaults:

- Tap repo: `homebrew-iris`
- Brew tap name: `sirius/iris`
- Publish URL: `htree://irischat/homebrew-iris`

To update a shared tap without deleting existing formulas, pass
`--seed-repo <existing-tap-url-or-path>`, or set the tap repo/push URL to an
existing htree tap and let `publish_tap.sh` clone it from the gateway first.
