#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

for loop_shell_script in scripts/*.sh; do
  bash -n "${loop_shell_script}"
done

./scripts/pnpm.sh exec biome check tests/toolchains tests/runtime
./scripts/pnpm.sh exec biome check infra/provider-egress.mjs
node --test --test-isolation=none tests/runtime/provider-boundaries.test.mjs
node --test --test-isolation=none tests/runtime/provider-egress.test.mjs
./scripts/uv.sh run --locked --offline --no-sync python scripts/check-function-names.py --self-test
./scripts/uv.sh run --locked --offline --no-sync python scripts/check-function-names.py
node --test --test-isolation=none tests/toolchains/function-names.test.mjs
node tests/toolchains/function-names.mjs --check
node --test --test-isolation=none tests/toolchains/rust-download.test.mjs
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
./scripts/uv-alphalens.sh run --locked --offline ruff check src tests
./scripts/uv-alphalens.sh run --locked --offline ruff format --check src tests
./scripts/uv-alphalens.sh run --locked --offline mypy
./scripts/uv-zipline.sh run --locked --offline ruff check src tests
./scripts/uv-zipline.sh run --locked --offline ruff format --check src tests
./scripts/uv-zipline.sh run --locked --offline mypy
git diff --check
