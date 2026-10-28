#!/usr/bin/env bash
set -Eeuo pipefail

PINS="$(cd "$(dirname "${BASH_SOURCE[0]}")/../build-inputs" && pwd)/cargo-deb.sha256"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ARCHIVE=cargo-deb-3.8.0.crate
curl --fail --silent --show-error --location --proto '=https' --max-time 120 --max-filesize 2097152 \
  "https://static.crates.io/crates/cargo-deb/$ARCHIVE" --output "$STAGE/$ARCHIVE"
EXPECTED="$(awk -v name="$ARCHIVE" '$2 == name { print $1 }' "$PINS")"
ACTUAL="$(sha256sum "$STAGE/$ARCHIVE")"
[[ "$EXPECTED" =~ ^[0-9a-f]{64}$ && "${ACTUAL%% *}" == "$EXPECTED" ]] || {
  echo 'cargo-deb archive integrity mismatch' >&2
  exit 1
}
# Verify before parsing the archive. Python extraction also avoids GNU tar's
# ENOSYS failure under the supported Docker Desktop amd64 emulation.
python3 -m tarfile -e "$STAGE/$ARCHIVE" "$STAGE"
(cd "$STAGE" && sha256sum --check --strict "$PINS")
cargo install --path "$STAGE/cargo-deb-3.8.0" --locked --force
