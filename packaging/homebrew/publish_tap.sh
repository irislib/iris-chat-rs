#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
usage: packaging/homebrew/publish_tap.sh --version <version> --release-base-url <url> --assets-dir <dir> [options]

Generate the Iris Homebrew formula and publish the tap repository.

Required:
  --version <version>              Release version, for example: v0.1.19
  --release-base-url <url>         Asset base URL containing iris-<target>.tar.gz files
  --assets-dir <dir>               Directory containing iris-<target>.tar.gz files

Optional:
  --tap-repo <name>                Published tap repo name (default: homebrew-iris)
  --tap-name <user/repo>           Brew tap name shown to users (default: sirius/iris)
  --push-url <url>                 Publish destination (default: htree://self/<tap-repo>)
  --npub <npub>                    Public npub used for the gateway install URL
  --seed-repo <url-or-path>        Existing tap repo to preserve before updating Formula/iris.rb
  --formula-name <name>            Formula name (default: iris)
  --homepage <url>                 Formula homepage
  --desc <text>                    Formula description
  --license <id>                   Formula license
  --no-merge-existing-tap          Do not auto-seed from an existing htree gateway tap
  --allow-fresh-shared-tap         Allow homebrew-hashtree without an existing seed repo
  -h, --help                       Show this help
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
CREATE_TAP_SCRIPT="$SCRIPT_DIR/create_tap.sh"

VERSION=""
RELEASE_BASE_URL=""
ASSETS_DIR=""
TAP_REPO="${IRIS_HOMEBREW_TAP_REPO:-homebrew-iris}"
TAP_NAME="${IRIS_HOMEBREW_TAP_NAME:-sirius/iris}"
PUSH_URL="${IRIS_HOMEBREW_TAP_PUSH_URL:-}"
NPUB=""
SEED_REPO=""
FORMULA_NAME="iris"
MERGE_EXISTING_TAP=1
ALLOW_FRESH_SHARED_TAP=0
CREATE_TAP_ARGS=()

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

default_htree_publish_name() {
  local name="$1"
  if [[ "$name" == *.git ]]; then
    printf '%s\n' "$name"
  else
    printf '%s.git\n' "$name"
  fi
}

htree_publish_name_from_url() {
  local url="$1"
  local name="${url#htree://}"
  name="${name#*/}"
  default_htree_publish_name "$name"
}

current_htree_npub() {
  htree user 2>/dev/null | grep -oE 'npub1[023456789acdefghjklmnpqrstuvwxyz]+' | head -n 1 || true
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|--release-base-url|--assets-dir|--formula-name|--homepage|--desc|--license)
      case "$1" in
        --version) VERSION="${2:-}" ;;
        --release-base-url) RELEASE_BASE_URL="${2:-}" ;;
        --assets-dir) ASSETS_DIR="${2:-}" ;;
        --formula-name) FORMULA_NAME="${2:-}" ;;
      esac
      CREATE_TAP_ARGS+=("$1" "${2:-}")
      shift 2
      ;;
    --tap-repo)
      TAP_REPO="${2:-}"
      shift 2
      ;;
    --tap-name)
      TAP_NAME="${2:-}"
      shift 2
      ;;
    --push-url)
      PUSH_URL="${2:-}"
      shift 2
      ;;
    --npub)
      NPUB="${2:-}"
      shift 2
      ;;
    --seed-repo)
      SEED_REPO="${2:-}"
      shift 2
      ;;
    --no-merge-existing-tap)
      MERGE_EXISTING_TAP=0
      shift
      ;;
    --allow-fresh-shared-tap)
      ALLOW_FRESH_SHARED_TAP=1
      shift
      ;;
    --output-dir)
      echo "--output-dir is managed internally by publish_tap.sh" >&2
      exit 1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$VERSION" || -z "$RELEASE_BASE_URL" || -z "$ASSETS_DIR" ]]; then
  usage >&2
  exit 1
fi

if [[ ! -d "$ASSETS_DIR" ]]; then
  echo "Assets directory does not exist: $ASSETS_DIR" >&2
  exit 1
fi

require_command git
require_command "$CREATE_TAP_SCRIPT"

if [[ -z "$PUSH_URL" ]]; then
  PUSH_URL="htree://self/${TAP_REPO}"
fi

if [[ -z "$NPUB" && "$PUSH_URL" == htree://* ]] && command -v htree >/dev/null 2>&1; then
  NPUB="$(current_htree_npub)"
fi

publish_name=""
gateway_url=""
if [[ "$PUSH_URL" == htree://* ]]; then
  publish_name="$(htree_publish_name_from_url "$PUSH_URL")"
  if [[ -n "$NPUB" ]]; then
    gateway_url="https://upload.iris.to/${NPUB}/${publish_name}"
  fi
fi

if [[ -z "$SEED_REPO" && "$MERGE_EXISTING_TAP" -eq 1 && -n "$gateway_url" ]]; then
  if git ls-remote "$gateway_url" >/dev/null 2>&1; then
    SEED_REPO="$gateway_url"
  elif [[ "$TAP_REPO" == "homebrew-hashtree" && "$ALLOW_FRESH_SHARED_TAP" -ne 1 ]]; then
    echo "Refusing to replace shared tap ${TAP_REPO}: existing gateway tap was not cloneable." >&2
    echo "Pass --seed-repo explicitly or --allow-fresh-shared-tap for the first shared-tap publish." >&2
    exit 1
  fi
fi

tmp_dir="$(mktemp -d)"
bare_repo="$tmp_dir/tap.git"
trap 'rm -rf "$tmp_dir"' EXIT

create_args=("${CREATE_TAP_ARGS[@]}" --output-dir "$bare_repo")
if [[ -n "$SEED_REPO" ]]; then
  create_args+=(--seed-repo "$SEED_REPO")
fi

"$CREATE_TAP_SCRIPT" "${create_args[@]}" >/dev/null

canonical_url=""
if [[ "$PUSH_URL" == htree://* ]]; then
  require_command htree
  (
    cd "$REPO_DIR"
    htree add "$bare_repo" --publish "$publish_name" >/dev/null
  )
  canonical_url="htree://self/${publish_name}"
else
  git --git-dir="$bare_repo" push -q --force "$PUSH_URL" master >/dev/null
fi

echo "Published Homebrew tap."

if [[ -n "$canonical_url" ]]; then
  cat <<EOF

Canonical:
  $canonical_url
EOF
fi

if [[ -n "$gateway_url" ]]; then
  cat <<EOF

Gateway URL:
  $gateway_url

Install:
  brew tap $TAP_NAME $gateway_url
  brew install $FORMULA_NAME
EOF
elif [[ -n "$TAP_NAME" ]]; then
  cat <<EOF

Install:
  brew tap $TAP_NAME <tap-url>
  brew install $FORMULA_NAME
EOF
fi
