#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/docker_stage_common.sh"

IMAGE="${IRIS_LINUX_BUILD_IMAGE:-$(docker_linux_dev_image)}"
PLATFORM="${IRIS_LINUX_DOCKER_PLATFORM:-$(docker_default_platform)}"
REPO_NAME="$(basename "$ROOT")"
RATCHET_ROOT="${IRIS_RATCHET_ROOT:-$ROOT/../nostr-double-ratchet}"
CARGO_CACHE="$ROOT/target/docker-stages/linux-idle-cpu-cargo"

docker_prepare_clean_config
trap docker_cleanup_clean_config EXIT
if [[ "${IRIS_LINUX_IDLE_CPU_REBUILD_IMAGE:-0}" == 1 ]] \
  || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  docker_build_image "$PLATFORM" \
    --build-arg INSTALL_CARGO_WATCH=0 \
    -f "$ROOT/linux/Dockerfile" -t "$IMAGE" "$ROOT/linux"
fi

mkdir -p "$CARGO_CACHE"
volumes=(
  -v "$ROOT:/workspace/$REPO_NAME:cached"
  -v "$CARGO_CACHE:/usr/local/cargo"
)
if [[ -d "$RATCHET_ROOT" ]]; then
  volumes+=(-v "$RATCHET_ROOT:/workspace/nostr-double-ratchet:cached")
fi

gate_command='./scripts/idle-cpu-platform-gate.sh --platform linux'
if [[ "${IRIS_LINUX_IDLE_CPU_SKIP_BUILD:-0}" == 1 ]]; then
  gate_command+=' --skip-build'
fi

docker run --platform "$PLATFORM" --rm \
  --user "$(id -u):$(id -g)" \
  "${volumes[@]}" \
  -w "/workspace/$REPO_NAME" \
  --entrypoint /bin/bash \
  -e HOME=/tmp \
  -e DISPLAY= \
  -e CARGO_INCREMENTAL=0 \
  -e CARGO_TARGET_DIR="/workspace/$REPO_NAME/linux/target" \
  -e IRIS_LINUX_IDLE_CPU_APP="/workspace/$REPO_NAME/linux/target/debug/iris-chat" \
  -e IRIS_CHAT_IDLE_CPU_MAX_PERCENT="${IRIS_CHAT_IDLE_CPU_MAX_PERCENT:-5}" \
  -e IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS="${IRIS_CHAT_IDLE_CPU_SETTLE_SECONDS:-30}" \
  -e IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS="${IRIS_CHAT_IDLE_CPU_SAMPLE_SECONDS:-60}" \
  "$IMAGE" -lc "$gate_command"
