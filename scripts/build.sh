#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

./scripts/cargo.sh build --locked --offline --workspace
./scripts/pnpm.sh build
./scripts/uv-research.sh run --locked --offline hatchling build
