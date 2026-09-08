#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-start}" in
  start)
    docker compose -f "${loop_repo_dir}/infra/compose/postgres-test.yaml" \
      up --detach --wait --wait-timeout 90 --build
    ;;
  stop)
    docker compose -f "${loop_repo_dir}/infra/compose/postgres-test.yaml" down
    ;;
  *) echo "usage: $0 [start|stop]" >&2; exit 2 ;;
esac
