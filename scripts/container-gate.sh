#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_gate_project="${LOOP_ENGINE_CONTAINER_GATE_PROJECT:-loop-engine-gate-$(id -u)-$(date +%s)-$$}"

cleanup() {
  docker compose --project-name "${loop_gate_project}" down \
    --volumes --remove-orphans >/dev/null 2>&1 || true
}

report_and_cleanup() {
  docker compose --project-name "${loop_gate_project}" exec -T postgres \
    df -k /var/lib/postgresql/data || true
  cleanup
}
trap report_and_cleanup EXIT

export LOOP_ENGINE_UID="${LOOP_ENGINE_UID:-$(id -u)}"
export LOOP_ENGINE_GID="${LOOP_ENGINE_GID:-$(id -g)}"

cd "${loop_repo_dir}"
node --test --test-isolation=none tests/runtime/isolation.test.mjs
cleanup
docker compose --project-name "${loop_gate_project}" build development postgres
docker compose --project-name "${loop_gate_project}" up --detach --wait --wait-timeout 90 postgres
docker compose --project-name "${loop_gate_project}" run --rm development ./scripts/bootstrap.sh
docker compose --project-name "${loop_gate_project}" run --rm development just check
docker compose --project-name "${loop_gate_project}" run --rm development just test
docker compose --project-name "${loop_gate_project}" run --rm development just build
docker compose --project-name "${loop_gate_project}" run --rm development just check
docker compose --project-name "${loop_gate_project}" run --rm development just doctor
