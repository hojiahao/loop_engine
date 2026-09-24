#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${loop_repo_dir}"

./scripts/verify-protocol-baseline.sh --self-test
./scripts/proto-generate.sh --check
./scripts/wire-fixtures.sh --check
node "${loop_repo_dir}/tests/contracts/audit/generate_action_binding_goldens.mjs" --check
./scripts/pnpm.sh exec buf breaking \
  --against "${loop_repo_dir}/fixtures/contracts/protocol/v1/schema.baseline.binpb"
./scripts/uv-protocol.sh run --locked --offline python \
  "${loop_repo_dir}/tests/contracts/check_protocol_boundaries.py" \
  "${loop_repo_dir}/fixtures/contracts/protocol/v1/schema.current.binpb"
echo "Protocol compatibility baseline passed."
