#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
usage: packaging/homebrew/create_tap.sh --version <version> --release-base-url <url> --assets-dir <dir> --output-dir <dir> [options]

Generate a Homebrew tap as a bare Git repository that can be published on a
static HTTP host.

Required:
  --version <version>              Release version, for example: v0.1.19
  --release-base-url <url>         Asset base URL containing iris-<target>.tar.gz files
  --assets-dir <dir>               Directory containing iris-<target>.tar.gz files
  --output-dir <dir>               Output directory for the bare tap repository

Optional:
  --seed-repo <url-or-path>        Existing tap repo to clone before updating Formula/iris.rb
  --formula-name <name>            Formula name (default: iris)
  --homepage <url>                 Formula homepage
  --desc <text>                    Formula description
  --license <id>                   Formula license (default: MIT)
  -h, --help                       Show this help

The generated formula installs the iris command line app.
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

VERSION=""
RELEASE_BASE_URL=""
ASSETS_DIR=""
OUTPUT_DIR=""
SEED_REPO=""
FORMULA_NAME="iris"
HOMEPAGE="https://git.iris.to/#/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/iris-chat-rs"
DESC="Iris Chat command line client"
LICENSE_ID="MIT"

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

formula_class_name() {
  local name="$1"
  awk -F'[-_]' '
    {
      for (i = 1; i <= NF; i++) {
        printf toupper(substr($i, 1, 1)) substr($i, 2)
      }
      printf "\n"
    }
  ' <<<"$name"
}

escape_ruby_string() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "$value"
}

checksum_for_target() {
  local target="$1"
  local file="$ASSETS_DIR/iris-${target}.tar.gz"

  if [[ ! -f "$file" ]]; then
    echo "Missing release archive: $file" >&2
    exit 1
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    echo "Missing required command: sha256sum or shasum" >&2
    exit 1
  fi
}

write_formula() {
  local output_file="$1"
  local class_name="$2"
  local formula_version="${VERSION#v}"
  local homepage_escaped desc_escaped license_escaped release_base_url_escaped
  local sha_macos_arm sha_macos_x86 sha_linux_x86

  homepage_escaped="$(escape_ruby_string "$HOMEPAGE")"
  desc_escaped="$(escape_ruby_string "$DESC")"
  license_escaped="$(escape_ruby_string "$LICENSE_ID")"
  release_base_url_escaped="$(escape_ruby_string "$RELEASE_BASE_URL")"
  sha_macos_arm="$(checksum_for_target "aarch64-apple-darwin")"
  sha_macos_x86="$(checksum_for_target "x86_64-apple-darwin")"
  sha_linux_x86="$(checksum_for_target "x86_64-unknown-linux-gnu")"

  cat > "$output_file" <<EOF
class ${class_name} < Formula
  desc "${desc_escaped}"
  homepage "${homepage_escaped}"
  version "${formula_version}"
  license "${license_escaped}"

  on_macos do
    if Hardware::CPU.arm?
      url "${release_base_url_escaped}/iris-aarch64-apple-darwin.tar.gz"
      sha256 "${sha_macos_arm}"
    else
      url "${release_base_url_escaped}/iris-x86_64-apple-darwin.tar.gz"
      sha256 "${sha_macos_x86}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      odie "Linux ARM64 Homebrew install is not available yet; use the release install script instead."
    else
      url "${release_base_url_escaped}/iris-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${sha_linux_x86}"
    end
  end

  def install
    bin.install "iris" => "iris"
  end

  test do
    assert_match "iris", shell_output("#{bin}/iris --help")
  end
end
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --release-base-url)
      RELEASE_BASE_URL="${2:-}"
      shift 2
      ;;
    --assets-dir)
      ASSETS_DIR="${2:-}"
      shift 2
      ;;
    --output-dir)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    --seed-repo)
      SEED_REPO="${2:-}"
      shift 2
      ;;
    --formula-name)
      FORMULA_NAME="${2:-}"
      shift 2
      ;;
    --homepage)
      HOMEPAGE="${2:-}"
      shift 2
      ;;
    --desc)
      DESC="${2:-}"
      shift 2
      ;;
    --license)
      LICENSE_ID="${2:-}"
      shift 2
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

if [[ -z "$VERSION" || -z "$RELEASE_BASE_URL" || -z "$ASSETS_DIR" || -z "$OUTPUT_DIR" ]]; then
  usage >&2
  exit 1
fi

if [[ ! -d "$ASSETS_DIR" ]]; then
  echo "Assets directory does not exist: $ASSETS_DIR" >&2
  exit 1
fi

require_command git
require_command awk

class_name="$(formula_class_name "$FORMULA_NAME")"
tmp_dir="$(mktemp -d)"
work_repo="$tmp_dir/homebrew-tap"
trap 'rm -rf "$tmp_dir"' EXIT

if [[ -n "$SEED_REPO" ]]; then
  git clone -q "$SEED_REPO" "$work_repo" >/dev/null
else
  mkdir -p "$work_repo"
  git -C "$work_repo" init -q -b master >/dev/null
fi

mkdir -p "$work_repo/Formula"
write_formula "$work_repo/Formula/${FORMULA_NAME}.rb" "$class_name"

(
  cd "$work_repo"
  git add "Formula/${FORMULA_NAME}.rb"
  if ! git diff --cached --quiet; then
    git -c user.name='Codex' -c user.email='codex@example.com' \
      commit -m "Update ${FORMULA_NAME} formula to ${VERSION}" >/dev/null
  fi
)

rm -rf "$OUTPUT_DIR"
git clone -q --bare "$work_repo" "$OUTPUT_DIR" >/dev/null
GIT_DIR="$OUTPUT_DIR" git update-server-info

cat <<EOF
Created bare tap repository:
  $OUTPUT_DIR

Formula:
  $FORMULA_NAME
EOF
