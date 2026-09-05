#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_local_rustup_home="${loop_runtime_root}/.tools/rustup"
export CARGO_HOME="${loop_runtime_root}/.tools/cargo"
export CARGO_TARGET_DIR="${loop_runtime_root}/.tools/target"
export PATH="${CARGO_HOME}/bin:${PATH}"

if [[ -d "${loop_local_rustup_home}/toolchains" ]]; then
  export RUSTUP_HOME="${loop_local_rustup_home}"
fi

exec cargo "$@"
