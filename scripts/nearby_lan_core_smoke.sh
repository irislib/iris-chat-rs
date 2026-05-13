#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${ROOT_DIR}/core"
cargo test -q desktop_lan_services_discover_each_other_on_same_host -- --ignored --nocapture
