#!/usr/bin/env bash

# Shared Docker build staging helpers. Scripts sourcing this file should set
# ROOT_DIR to the repository root first.

docker_stage_root() {
  printf '%s\n' "${IRIS_DOCKER_STAGE_ROOT:-${ROOT_DIR}/target/docker-stages}"
}

docker_default_platform() {
  printf '%s\n' "${IRIS_DOCKER_PLATFORM:-linux/amd64}"
}

docker_cli_runtime_image() {
  printf '%s\n' "${IRIS_CLI_DOCKER_IMAGE:-debian:bookworm-slim}"
}

docker_rust_build_image() {
  printf '%s\n' "${IRIS_RUST_DOCKER_IMAGE:-rust:1-bookworm}"
}

docker_linux_dev_image() {
  printf '%s\n' "${IRIS_LINUX_DOCKER_IMAGE:-iris-chat-linux-dev:latest}"
}

docker_stage_dir() {
  local name="$1"
  printf '%s/%s\n' "$(docker_stage_root)" "$name"
}

docker_stage_build_iris_cli() {
  local stage_dir="$1"
  local platform="$2"
  local image="$3"
  local rebuild="$4"
  local src_root="$5"

  if [[ "${rebuild}" == "1" ]]; then
    rm -rf "${stage_dir}"
  fi
  mkdir -p "${stage_dir}"

  echo "Building current checkout in ${image}"
  echo "Using staged build cache: ${stage_dir}"
  docker run --rm --platform "${platform}" \
    -v "${src_root}:/work:ro" \
    -v "${stage_dir}:/stage" \
    -e CARGO_HOME=/stage/cargo \
    -e CARGO_TARGET_DIR=/stage/target \
    "${image}" \
    sh -lc '
      set -eu
      export PATH="/usr/local/cargo/bin:$PATH"
      export RUSTUP_HOME="${RUSTUP_HOME:-/usr/local/rustup}"
      cargo build --manifest-path /work/iris-chat-rs/core/Cargo.toml --bin iris --locked
      cp /stage/target/debug/iris /stage/iris
    '

  DOCKER_STAGE_IRIS_CLI_BIN="${stage_dir}/iris"
}
