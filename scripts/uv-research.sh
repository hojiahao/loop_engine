#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_project_dir="${loop_repo_dir}/python/loop_research"

cd "${loop_project_dir}"
if [[ "${1:-}" != "run" ]]; then
  echo "uv-research.sh supports only 'run'; use scripts/uv.sh for workspace operations" >&2
  exit 2
fi

shift
exec "${loop_repo_dir}/scripts/uv.sh" run --package loop-research "$@"
