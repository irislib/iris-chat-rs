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

use_snapshot_archive() {
  local state="$1" distribution="$2" digest="$3" archive="$1/snapshot.tar.gz"
  shift 3
  echo "Using the verified $distribution snapshot mirror." >&2
  /usr/lib/apt/apt-helper "$@" download-file \
    "https://upload.iris.to/$digest.bin" "$archive" "SHA256:$digest"
  mkdir "$state/mirror"
  tar -xzf "$archive" -C "$state/mirror" --no-same-owner --no-same-permissions
  chmod -R a+rX "$state/mirror"
  # Start again with empty lists so a partial primary update cannot supply any index.
  mv "$state/lists" "$state/primary-lists"
  mkdir -m 755 -p "$state/lists/partial"
  mv "$state/mirror.sources.list" "$state/sources.list"
  apt-get "$@" update
  verify_lists "$distribution" "$state/lists"
}

main() (
  local distribution="${1:-}" keyring profile suite uri expected components count=0 state
  local package version installed selection mirror_digest= source_uri status
  [[ $# -ge 2 ]] || { echo 'usage: pinned-apt.sh ubuntu|ubuntu-noble|debian package ...' >&2; exit 2; }
  shift
  case "$distribution" in
    ubuntu|ubuntu-noble) keyring=/usr/share/keyrings/ubuntu-archive-keyring.gpg ;;
    debian) keyring=/usr/share/keyrings/debian-archive-keyring.gpg ;;
    *) echo 'Unsupported snapshot distribution' >&2; exit 2 ;;
  esac
  # These archives contain only amd64 indexes from the committed Ubuntu snapshot.
  if [[ "$distribution" != debian && "$(dpkg --print-architecture)" == amd64 && -z "$(dpkg --print-foreign-architectures)" ]]; then
    case "$distribution" in
      ubuntu-noble) mirror_digest=01e5a5305b6c6138f5938ca287ee36f19ed859931be446feb4588facaf581b03 ;;
      ubuntu) mirror_digest=62f52a3021f171e53cd73f41b8ea457bac48f4a6f3a76051af5d031992e9d1b8 ;;
    esac
  fi
  state="$(mktemp -d)"
  trap 'rm -rf "$state"' EXIT
  mkdir -p "$state/lists/partial" "$state/archives/partial"
  chmod 755 "$state" "$state/lists" "$state/archives"
  while IFS=$'\t' read -r profile suite uri expected components; do
    [[ "$profile" == "$distribution" ]] || continue
    [[ "$expected" =~ ^[0-9a-f]{64}$ && "$suite" =~ ^[a-z-]+$ ]] || exit 1
    source_uri="$uri"
    if [[ -n "$mirror_digest" ]]; then
      # During metadata acquisition each list contains exactly one origin.
      printf '%s/\n' "$uri" > "$state/$suite.primary"
      source_uri="mirror+file:$state/$suite.primary"
      printf 'file:%s/mirror/\n' "$state" > "$state/$suite.secondary"
      printf 'deb [signed-by=%s check-valid-until=no target=Packages] mirror+file:%s/%s.secondary %s %s\n' \
        "$keyring" "$state" "$suite" "$suite" "$components" >> "$state/mirror.sources.list"
    fi
    printf 'deb [signed-by=%s check-valid-until=no] %s %s %s\n' \
      "$keyring" "$source_uri" "$suite" "$components" >> "$state/sources.list"
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
  if apt-get "${options[@]}" update; then
    verify_lists "$distribution" "$state/lists"
  else
    status=$?
    [[ "$status" == 100 && -n "$mirror_digest" ]] || exit "$status"
    use_snapshot_archive "$state" "$distribution" "$mirror_digest" "${options[@]}"
  fi
  # The trusted indexes are now local. Permit the ordinary Ubuntu pool to
  # supply .debs only for the install, keeping source-list identities stable.
  if [[ -n "$mirror_digest" ]]; then
    while IFS=$'\t' read -r profile suite uri expected components; do
      [[ "$profile" == "$distribution" ]] || continue
      if [[ -d "$state/mirror" ]]; then
        printf 'https://archive.ubuntu.com/ubuntu/\tpriority:1 type:deb\n%s/\tpriority:2 type:deb\n' "$uri" > "$state/$suite.secondary"
      else
        printf '%s/\tpriority:1 type:deb\nhttps://archive.ubuntu.com/ubuntu/\tpriority:2 type:deb\n' "$uri" > "$state/$suite.primary"
      fi
    done < "$PIN_FILE"
  fi
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
