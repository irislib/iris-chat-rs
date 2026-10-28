#!/usr/bin/env bash
# Only the reviewed snapshot metadata may select packages for these containers.
set -Eeuo pipefail

PIN_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../build-inputs" && pwd)/apt-snapshots.tsv"

verify_lists() {
  local distribution="$1" lists="$2" profile suite uri expected components actual
  local checked=0
  local -a matches releases
  shopt -s nullglob
  releases=("$lists/"*_InRelease)
  [[ "${#releases[@]}" == 3 ]] || { echo 'Expected exactly three signed snapshot roots' >&2; return 1; }
  while IFS=$'\t' read -r profile suite uri expected components; do
    [[ "$profile" == "$distribution" ]] || continue
    matches=("$lists/"*"_dists_${suite}_InRelease")
    [[ "${#matches[@]}" == 1 ]] || { echo "Missing snapshot root: $suite" >&2; return 1; }
    actual="$(sha256sum "${matches[0]}")"
    [[ "${actual%% *}" == "$expected" ]] || { echo "Snapshot integrity mismatch: $suite" >&2; return 1; }
    checked=$((checked + 1))
  done < "$PIN_FILE"
  [[ "$checked" == 3 ]]
}

main() (
  local distribution="${1:-}" keyring profile suite uri expected components count=0 state
  local package version installed selection
  [[ $# -ge 2 ]] || { echo 'usage: pinned-apt.sh ubuntu|ubuntu-noble|debian package ...' >&2; exit 2; }
  shift
  case "$distribution" in
    ubuntu|ubuntu-noble) keyring=/usr/share/keyrings/ubuntu-archive-keyring.gpg ;;
    debian) keyring=/usr/share/keyrings/debian-archive-keyring.gpg ;;
    *) echo 'Unsupported snapshot distribution' >&2; exit 2 ;;
  esac
  state="$(mktemp -d)"
  trap 'rm -rf "$state"' EXIT
  mkdir -p "$state/lists/partial" "$state/archives/partial"
  chmod 755 "$state" "$state/lists" "$state/archives"
  while IFS=$'\t' read -r profile suite uri expected components; do
    [[ "$profile" == "$distribution" ]] || continue
    [[ "$expected" =~ ^[0-9a-f]{64}$ && "$suite" =~ ^[a-z-]+$ ]] || exit 1
    printf 'deb [signed-by=%s check-valid-until=no] %s %s %s\n' \
      "$keyring" "$uri" "$suite" "$components" >> "$state/sources.list"
    count=$((count + 1))
  done < "$PIN_FILE"
  [[ "$count" == 3 ]] || { echo 'Incomplete snapshot pins' >&2; exit 1; }
  local -a options=(
    -o "Dir::Etc::sourcelist=$state/sources.list" -o Dir::Etc::sourceparts=-
    -o "Dir::State::lists=$state/lists" -o "Dir::Cache::archives=$state/archives"
    -o "Dir::Cache::pkgcache=$state/pkgcache.bin" -o "Dir::Cache::srcpkgcache=$state/srcpkgcache.bin"
    -o Dir::Etc::preferences=- -o Dir::Etc::preferencesparts=- -o APT::Default-Release=
    -o Acquire::Languages=none -o APT::Update::Error-Mode=any
    -o Acquire::AllowInsecureRepositories=false -o Acquire::AllowDowngradeToInsecureRepositories=false
    -o APT::Get::AllowUnauthenticated=false -o APT::Get::allow-downgrades=false
  )
  # APT must verify signatures, then these fixed InRelease hashes bind Packages
  # and .deb hashes, including on the Debian snapshot's HTTP transport.
  apt-get "${options[@]}" update
  verify_lists "$distribution" "$state/lists"
  local -a selected=()
  for package in "$@"; do
    [[ "$package" =~ ^[a-z0-9][a-z0-9+.-]*(:[a-z0-9-]+)?$ ]] || {
      echo "Expected a bare package name: $package" >&2; exit 1;
    }
    # Exclude installed status and binary caches: a newer hosted package must
    # not become the candidate merely because it is already present.
    version="$(LC_ALL=C apt-cache "${options[@]}" -o Dir::State::status=/dev/null \
      -o Dir::Cache::pkgcache= -o Dir::Cache::srcpkgcache= policy "$package" \
      | awk '/^[[:space:]]*Candidate:/ { print $2 }')"
    [[ "$version" =~ ^[0-9][a-zA-Z0-9.+:~_-]*$ ]] || {
      echo "No version in the verified snapshot: $package" >&2; exit 1;
    }
    installed="$(dpkg-query -W -f='${Status}\t${Version}' "$package" 2>/dev/null || true)"
    if [[ "$installed" == $'install ok installed\t'* ]] && \
       dpkg --compare-versions "${installed#*$'\t'}" gt "$version"; then
      echo "Installed $package is newer than snapshot $version; refusing a downgrade" >&2
      exit 1
    fi
    selected+=("$package=$version")
  done
  # Reinstall requested roots even at the same version, so their executable
  # bytes come from the verified package chain, not an ambient modification.
  DEBIAN_FRONTEND=noninteractive apt-get "${options[@]}" install -y --no-install-recommends \
    --no-remove --reinstall -- "${selected[@]}"
  for selection in "${selected[@]}"; do
    package="${selection%%=*}"; version="${selection#*=}"
    installed="$(dpkg-query -W -f='${Status}\t${Version}' "$package")"
    [[ "$installed" == $'install ok installed\t'"$version" ]] || {
      echo "Installed package differs from snapshot: $selection" >&2; exit 1;
    }
    printf 'Verified snapshot package: %s\n' "$selection"
  done
)

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
