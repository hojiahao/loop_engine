#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
export UV_CACHE_DIR="${loop_runtime_root}/.tools/uv-cache"
export UV_PYTHON_INSTALL_DIR="${loop_runtime_root}/.tools/python"
unset UV_PROJECT_ENVIRONMENT
loop_python_version="$(tr -d '[:space:]' < "${loop_repo_dir}/.python-version")"

if [[ "${1:-}" != "run" ]]; then
  echo "uv-alphalens.sh supports only isolated run; it never creates another .venv" >&2
  exit 2
fi
shift
cd "${loop_repo_dir}/python/alphalens_validation"
exec uv run --project . --python "${loop_python_version}" --isolated "$@"
