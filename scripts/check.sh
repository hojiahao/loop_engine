#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

./scripts/cargo.sh fmt --all -- --check
./scripts/cargo.sh clippy --locked --offline --workspace --all-targets -- -D warnings
./scripts/pnpm.sh check
./scripts/uv-research.sh sync --locked --offline
./scripts/uv-research.sh run --locked --offline ruff check src tests
./scripts/uv-research.sh run --locked --offline ruff format --check src tests
./scripts/uv-research.sh run --locked --offline mypy
git diff --check
