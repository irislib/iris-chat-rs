#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT_DIR}/scripts/mobile_relay_common.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT
FAKE_EMULATOR="${TMP_DIR}/emulator"

cat >"${FAKE_EMULATOR}" <<'EOF'
#!/usr/bin/env bash
if [[ "$1" == "-list-avds" ]]; then
  printf '%s\n' Preferred Fallback_A Fallback_B
fi
EOF
chmod +x "${FAKE_EMULATOR}"

selected="$(android_select_installed_avds "${FAKE_EMULATOR}" 2 Missing Preferred)"
[[ "${selected}" == $'Preferred\nFallback_A' ]]

selected="$(android_select_installed_avds "${FAKE_EMULATOR}" 4 Missing Preferred)"
[[ "${selected}" == $'Preferred\nFallback_A\nFallback_B\nPreferred' ]]

cat >"${FAKE_EMULATOR}" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "${FAKE_EMULATOR}"

output="$(android_select_installed_avds "${FAKE_EMULATOR}" 1 Missing || true)"
[[ -z "${output}" ]]

echo "mobile relay AVD selection harness passed"
