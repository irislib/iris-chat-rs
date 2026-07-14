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

docker_prepare_clean_config() {
  if [[ "${IRIS_DOCKER_CLEAN_CONFIG:-0}" != "1" || -n "${DOCKER_CONFIG:-}" ]]; then
    return 0
  fi

  DOCKER_CLEAN_CONFIG_DIR="$(mktemp -d /tmp/iris-docker-config.XXXXXX)"
  printf '%s\n' '{}' > "${DOCKER_CLEAN_CONFIG_DIR}/config.json"
  export DOCKER_CONFIG="${DOCKER_CLEAN_CONFIG_DIR}"

  local plugin
  for plugin in \
    "${HOME}/.docker/cli-plugins/docker-buildx" \
    "/Applications/Docker.app/Contents/Resources/cli-plugins/docker-buildx"
  do
    if [[ -e "${plugin}" ]]; then
      mkdir -p "${DOCKER_CLEAN_CONFIG_DIR}/cli-plugins"
      ln -s "${plugin}" "${DOCKER_CLEAN_CONFIG_DIR}/cli-plugins/docker-buildx" 2>/dev/null || true
      break
    fi
  done

  if [[ -z "${DOCKER_HOST:-}" && -S "${HOME}/.docker/run/docker.sock" ]]; then
    export DOCKER_HOST="unix://${HOME}/.docker/run/docker.sock"
  fi
}

docker_cleanup_clean_config() {
  if [[ -n "${DOCKER_CLEAN_CONFIG_DIR:-}" ]]; then
    rm -rf "${DOCKER_CLEAN_CONFIG_DIR}"
  fi
}

docker_build_image() {
  local platform="$1"
  shift

  if docker buildx version >/dev/null 2>&1; then
    docker buildx build --load --platform "${platform}" "$@"
  else
    docker build --platform "${platform}" "$@"
  fi
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
    -v "${src_root}:/work/iris-chat-rs:ro" \
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
