#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
cd "${loop_repo_dir}"

./scripts/cargo.sh test --locked --offline --workspace
./scripts/pnpm.sh test
./scripts/uv-research.sh run --locked --offline pytest

UV_CACHE_DIR="${loop_runtime_root}/.tools/uv-cache" \
UV_PYTHON_INSTALL_DIR="${loop_runtime_root}/.tools/python" \
UV_PROJECT_ENVIRONMENT="${loop_runtime_root}/.venv-legacy" \
  uv run --project "${loop_repo_dir}" --locked --offline pytest
