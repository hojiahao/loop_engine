#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
export COREPACK_HOME="${COREPACK_HOME:-${loop_runtime_root}/.tools/corepack}"
export PNPM_HOME="${loop_runtime_root}/.tools/pnpm-home"
export XDG_CACHE_HOME="${loop_runtime_root}/.tools/xdg-cache"

exec corepack pnpm \
  --config.store-dir="${loop_runtime_root}/.tools/pnpm-store" \
  --config.package-import-method=copy \
  "$@"
