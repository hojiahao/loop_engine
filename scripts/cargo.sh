#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
source "${loop_repo_dir}/scripts/rust-toolchain-common.sh"
loop_rust_metadata_init "${loop_repo_dir}" "${loop_runtime_root}"
export CARGO_HOME="${loop_runtime_root}/.tools/cargo"
export CARGO_TARGET_DIR="${loop_runtime_root}/.tools/target"

if [[ -e "${loop_rust_local_prefix}" ]]; then
  if ! loop_rust_prefix_is_valid "${loop_rust_local_prefix}"; then
    echo "local Rust toolchain identity validation failed; run ./scripts/bootstrap.sh" >&2
    exit 2
  fi
  loop_active_rust_prefix="${loop_rust_local_prefix}"
else
  loop_system_prefix="$(loop_system_rust_prefix || true)"
  if [[ -z "${loop_system_prefix}" ]] || ! loop_rust_prefix_is_valid "${loop_system_prefix}"; then
    echo "no verified Rust toolchain is active; run ./scripts/bootstrap.sh" >&2
    exit 2
  fi
  loop_active_rust_prefix="${loop_system_prefix}"
fi

export PATH="${loop_active_rust_prefix}/bin:${CARGO_HOME}/bin:${PATH}"

exec cargo "$@"
