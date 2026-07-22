#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

BROKEN_UDID="11111111-1111-1111-1111-111111111111"
HEALTHY_UDID="22222222-2222-2222-2222-222222222222"
CLONE_UDID="33333333-3333-3333-3333-333333333333"
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
{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[{"name":"Iris Chat iPhone","udid":"${FAKE_BROKEN_UDID}","isAvailable":true},{"name":"Iris Chat iPhone","udid":"${FAKE_HEALTHY_UDID}","isAvailable":true}]}}
JSON
elif [[ "$*" == "simctl shutdown ${FAKE_BROKEN_UDID}" || "$*" == "simctl shutdown ${FAKE_HEALTHY_UDID}" ]]; then
  :
elif [[ "$*" == simctl\ clone\ "${FAKE_BROKEN_UDID}"\ Iris\ clone\ probe* ]]; then
  echo "clone source is unhealthy" >&2
  exit 2
elif [[ "$*" == simctl\ clone\ "${FAKE_HEALTHY_UDID}"\ Iris\ clone\ probe* ]]; then
  echo "${FAKE_CLONE_UDID}"
elif [[ "$*" == "simctl delete ${FAKE_CLONE_UDID}" ]]; then
  :
elif [[ "$*" == "simctl boot ${FAKE_HEALTHY_UDID}" ]]; then
  touch "${FAKE_BOOTED_MARKER}"
elif [[ "$*" == "simctl bootstatus ${FAKE_HEALTHY_UDID} -b" ]]; then
  :
elif [[ "$*" == "simctl list devices booted" ]]; then
  if [[ -f "${FAKE_BOOTED_MARKER}" ]]; then
    echo "    Iris Chat iPhone (${FAKE_HEALTHY_UDID}) (Booted)"
  fi
else
  echo "unexpected fake xcrun invocation: $*" >&2
  exit 99
fi
EOF
chmod +x "${TMP_DIR}/xcrun"

destination="$({
  PATH="${TMP_DIR}:${PATH}" \
    FAKE_XCRUN_LOG="${LOG}" \
    FAKE_BROKEN_UDID="${BROKEN_UDID}" \
    FAKE_HEALTHY_UDID="${HEALTHY_UDID}" \
    FAKE_CLONE_UDID="${CLONE_UDID}" \
    FAKE_BOOTED_MARKER="${TMP_DIR}/booted" \
    IRIS_E2E_CLOSE_STALE_IOS_SIMS=0 \
    IRIS_IOS_REQUIRE_CLONEABLE=1 \
    "${ROOT}/scripts/ios-simulator-destination"
})"

[[ "${destination}" == "platform=iOS Simulator,id=${HEALTHY_UDID}" ]]
grep -q "simctl clone ${BROKEN_UDID}" "${LOG}"
grep -q "simctl clone ${HEALTHY_UDID}" "${LOG}"
grep -q "simctl delete ${CLONE_UDID}" "${LOG}"
! grep -q "simctl boot ${BROKEN_UDID}" "${LOG}"
grep -q "simctl boot ${HEALTHY_UDID}" "${LOG}"

echo "iOS cloneable simulator selection harness passed"
