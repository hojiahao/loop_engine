#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

./scripts/cargo.sh test --locked --offline --workspace --all-features
./scripts/pnpm.sh test
./scripts/uv.sh sync --all-packages --all-groups --locked --offline
./scripts/verify-python-environment.sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest
./scripts/uv.sh run --isolated --locked --offline pytest
