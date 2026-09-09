#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

node --test --test-isolation=none tests/toolchains/rust-download.test.mjs
if [[ -z "${LOOP_TEST_POSTGRES_URL:-}" ]]; then
  mkdir -p .tools
  exec 9>.tools/postgres-test.lock
  if ! flock --nonblock 9; then
    echo "Another managed test suite owns the PostgreSQL fixture." >&2
    exit 1
  fi
  trap 'bash ./scripts/postgres-test.sh stop' EXIT
  bash ./scripts/postgres-test.sh stop
  bash ./scripts/postgres-test.sh start
fi
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
./scripts/cargo.sh test --locked --offline --workspace --all-features
./scripts/pnpm.sh test
./scripts/uv.sh sync --all-packages --all-groups --locked --offline
./scripts/verify-python-environment.sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest
./scripts/uv.sh run --isolated --locked --offline pytest
