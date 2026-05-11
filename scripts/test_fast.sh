#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"${ROOT_DIR}/scripts/check-palettes"
"${ROOT_DIR}/scripts/check-no-accent-text"
"${ROOT_DIR}/scripts/test_rust.sh"
"${ROOT_DIR}/scripts/local_relay_scenario_soak.sh" --iterations 1
(cd "${ROOT_DIR}/android" && ./gradlew :app:compileDebugKotlin :app:compileDebugAndroidTestKotlin)
"${ROOT_DIR}/scripts/ios-build" ios-test
