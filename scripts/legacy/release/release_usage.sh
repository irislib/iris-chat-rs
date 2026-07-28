#!/usr/bin/env bash

usage() {
  cat <<'EOF'
usage: ./scripts/release [options]

Build all locally-buildable platforms, stage a hashtree release directory under
dist/release/<tag>/, and optionally publish it via htree.

Options:
  --publish                Publish the staged tree and repoint the htree latest release
  --allow-partial-latest   Allow --publish to repoint latest even if required steps are missing
  --skip-zapstore          Do not publish the signed Android APK to Zapstore during --publish
  --publish-zapstore       Publish the signed Android APK to Zapstore, even without --publish
  --publish-cargo-crate    Publish the iris-chat Rust crate if this version is new
  --skip-cargo-crate       Do not publish the iris-chat Rust crate
  --skip-homebrew-tap      Do not update the Homebrew tap during --publish
  --homebrew-tap-repo <n>  htree tap repo name (default: homebrew-iris)
  --homebrew-tap-name <n>  brew tap name shown to users (default: sirius/iris)
  --homebrew-tap-push-url <url>
                           Override Homebrew tap publish destination
  --testflight             Upload iOS and attach internal + public TestFlight groups
  --testflight-internal    Upload iOS and attach internal TestFlight groups
  --testflight-public      Upload iOS and submit/attach public TestFlight groups
  --skip-testflight        Do not send the iOS build to TestFlight
  --gate <mode>            Release gate mode: local, full, on-device, none
  --skip-gate              Do not run the release gate
  --force-gate             Run the gate even if this commit has a recent matching receipt
  --resume-staged          Resume from an exact, validated staged build without rebuilding
  --tag <vX.Y.Z>           Override release tag (defaults to v$IRIS_APP_VERSION_NAME)
  --release-tree <name>    htree release tree name (default: releases/iris-chat-rs)
  --only <csv>             Only run named steps (macos,windows,android,linux,cli,ios)
  --skip <csv>             Skip named steps
  --dry-run                Print the plan without running build commands
  -h, --help               Show this help

Environment:
  IRIS_RELEASE_TREE         Default for --release-tree
  IRIS_CLI_TARGETS          Comma-separated CLI targets for the iris binary
  IRIS_RELEASE_ALLOW_DIRTY  Allow building from a dirty git tree
  IRIS_RELEASE_ALLOW_PARTIAL_LATEST
                            Allow partial releases to repoint latest
  IRIS_RELEASE_REQUIRED_LATEST_STEPS
                            Required steps for --publish latest (default: macos,windows,android,linux,cli)
  IRIS_RELEASE_GATE_ARGS    Extra/default args for scripts/test-release-gate
  IRIS_RELEASE_GATE_FORCE_RERUN
  IRIS_RELEASE_TESTFLIGHT   both, internal, public, or 0
  IRIS_RELEASE_PUBLISH_ZAPSTORE
  IRIS_RELEASE_SKIP_ZAPSTORE
  IRIS_RELEASE_SKIP_HOMEBREW_TAP
  IRIS_HOMEBREW_TAP_REPO    Default Homebrew tap repo (default: homebrew-iris)
  IRIS_HOMEBREW_TAP_NAME    Default brew tap name (default: sirius/iris)
  IRIS_HOMEBREW_TAP_PUSH_URL
EOF
}
