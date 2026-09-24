#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_mode="${1:---check}"

if [[ "${loop_mode}" != "--write" && "${loop_mode}" != "--check" ]]; then
  echo "usage: $0 [--write|--check]" >&2
  exit 2
fi

mkdir -p "${loop_runtime_root}/.tools"
loop_staging_dir="$(mktemp -d "${loop_runtime_root}/.tools/wire-fixtures.XXXXXX")"
cleanup() {
  rm -rf -- "${loop_staging_dir}"
}
trap cleanup EXIT

cd "${loop_repo_dir}"
./scripts/cargo.sh run --locked --offline --quiet \
  -p loop-protocol --bin generate_wire_fixture -- \
  "${loop_staging_dir}/protocol_info_v1.rust.binpb"
./scripts/pnpm.sh --filter @loop-engine/protocol build
node "${loop_repo_dir}/packages/protocol-ts/dist/bin/generate-wire-fixture.js" \
  "${loop_staging_dir}/protocol_info_v1.typescript.binpb"
./scripts/uv-protocol.sh run --locked --offline python \
  "${loop_repo_dir}/tests/contracts/protocol/generate_wire_fixtures.py" \
  --output-dir "${loop_staging_dir}"

loop_protocol_fixture_dir="${loop_repo_dir}/fixtures/contracts/protocol/v1"
loop_fixture_names=(
  job_specification_v1_unknown_enum.binpb
  job_specification_v1_unknown_oneof.binpb
  protocol_info_v1.binpb
  protocol_info_v1.rust.binpb
  protocol_info_v1.typescript.binpb
  protocol_info_v1_unknown_field.binpb
  wire_fixtures.json
)

if [[ "${loop_mode}" == "--write" ]]; then
  mkdir -p "${loop_protocol_fixture_dir}"
  for loop_fixture_name in "${loop_fixture_names[@]}"; do
    install -m 0644 "${loop_staging_dir}/${loop_fixture_name}" \
      "${loop_protocol_fixture_dir}/${loop_fixture_name}"
  done
  install -m 0644 "${loop_staging_dir}/operational_failure.json" \
    "${loop_repo_dir}/tests/contracts/operational_failure.json"
  echo "Cross-language wire fixtures were regenerated."
  exit 0
fi

loop_mismatches=()
for loop_fixture_name in "${loop_fixture_names[@]}"; do
  if ! cmp -s "${loop_staging_dir}/${loop_fixture_name}" \
    "${loop_protocol_fixture_dir}/${loop_fixture_name}"; then
    loop_mismatches+=("fixtures/contracts/protocol/v1/${loop_fixture_name}")
  fi
done
if ! cmp -s "${loop_staging_dir}/operational_failure.json" \
  "${loop_repo_dir}/tests/contracts/operational_failure.json"; then
  loop_mismatches+=("tests/contracts/operational_failure.json")
fi
if (( ${#loop_mismatches[@]} > 0 )); then
  printf 'wire fixtures are missing or stale: %s\n' "${loop_mismatches[*]}" >&2
  exit 1
fi
echo "Cross-language wire fixtures match their pinned producers."
