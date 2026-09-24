#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"

export UV_CACHE_DIR="${loop_runtime_root}/.tools/uv-cache"
export UV_PYTHON_INSTALL_DIR="${loop_runtime_root}/.tools/python"
export UV_PROJECT_ENVIRONMENT="${loop_repo_dir}/.venv"

exec uv --project "${loop_repo_dir}" "$@"
