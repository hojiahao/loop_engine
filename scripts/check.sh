#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

for loop_shell_script in scripts/*.sh; do
  bash -n "${loop_shell_script}"
done

./scripts/proto-check.sh
./scripts/cargo.sh fmt --all -- --check
./scripts/cargo.sh clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
./scripts/pnpm.sh check
./scripts/uv.sh sync --all-packages --all-groups --locked --offline
./scripts/verify-python-environment.sh
./scripts/uv-research.sh run --locked --offline --no-sync ruff check src tests
./scripts/uv-research.sh run --locked --offline --no-sync ruff format --check src tests
./scripts/uv-research.sh run --locked --offline --no-sync mypy
./scripts/uv-protocol.sh run --locked --offline --no-sync ruff check src tests
./scripts/uv-protocol.sh run --locked --offline --no-sync ruff format --check src tests
./scripts/uv-protocol.sh run --locked --offline --no-sync mypy
git diff --check
