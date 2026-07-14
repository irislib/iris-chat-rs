#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

STALE_UDID="11111111-1111-1111-1111-111111111111"
FRESH_UDID="22222222-2222-2222-2222-222222222222"
LOG="${TMP_DIR}/xcrun.log"

cat > "${TMP_DIR}/xcrun" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
echo "$*" >> "${FAKE_XCRUN_LOG}"
if [[ "$*" == "simctl list -j devicetypes runtimes devices" ]]; then
  cat <<JSON
{"runtimes":[{"isAvailable":true,"identifier":"com.apple.CoreSimulator.SimRuntime.iOS-26-5","name":"iOS 26.5"}],"devicetypes":[{"identifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-16","name":"iPhone 16"}],"devices":{}}
JSON
elif [[ "$*" == "simctl list -j devices" ]]; then
  cat <<JSON
{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[{"name":"Iris Chat iPhone","udid":"${FAKE_STALE_UDID}","isAvailable":true}]}}
JSON
elif [[ "$*" == "simctl list devices booted" ]]; then
  if [[ -f "${FAKE_BOOTED_MARKER}" ]]; then
    echo "    Iris Chat iPhone (${FAKE_FRESH_UDID}) (Booted)"
  fi
elif [[ "$*" == "simctl boot ${FAKE_STALE_UDID}" ]]; then
  echo "Unable to boot deleted device" >&2
  exit 44
elif [[ "$*" == "simctl create Iris Chat iPhone com.apple.CoreSimulator.SimDeviceType.iPhone-16 com.apple.CoreSimulator.SimRuntime.iOS-26-5" ]]; then
  echo "${FAKE_FRESH_UDID}"
elif [[ "$*" == "simctl boot ${FAKE_FRESH_UDID}" ]]; then
  touch "${FAKE_BOOTED_MARKER}"
elif [[ "$*" == "simctl bootstatus ${FAKE_FRESH_UDID} -b" ]]; then
  :
else
  echo "unexpected fake xcrun invocation: $*" >&2
  exit 99
fi
EOF
chmod +x "${TMP_DIR}/xcrun"

destination="$({
  PATH="${TMP_DIR}:${PATH}" \
    FAKE_XCRUN_LOG="${LOG}" \
    FAKE_STALE_UDID="${STALE_UDID}" \
    FAKE_FRESH_UDID="${FRESH_UDID}" \
    FAKE_BOOTED_MARKER="${TMP_DIR}/booted" \
    IRIS_E2E_CLOSE_STALE_IOS_SIMS=0 \
    "${ROOT}/scripts/ios-simulator-destination"
})"

[[ "${destination}" == "platform=iOS Simulator,id=${FRESH_UDID}" ]]
grep -q "simctl boot ${STALE_UDID}" "${LOG}"
! grep -q "simctl delete" "${LOG}"
grep -q "simctl create Iris Chat iPhone" "${LOG}"
grep -q "simctl bootstatus ${FRESH_UDID} -b" "${LOG}"

echo "iOS simulator recovery harness passed"
