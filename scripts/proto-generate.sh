#!/usr/bin/env bash
set -euo pipefail

# Generated Python packages are source artifacts. Runtime bytecode is local
# cache state and must neither enter the staged tree nor affect drift checks.
export PYTHONDONTWRITEBYTECODE=1

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_mode="${1:---write}"

if [[ "${loop_mode}" != "--write" && "${loop_mode}" != "--check" && \
  "${loop_mode}" != "--initialize-baseline" ]]; then
  echo "usage: $0 [--write|--check|--initialize-baseline]" >&2
  exit 2
fi

mkdir -p "${loop_runtime_root}/.tools"
loop_staging_dir="$(mktemp -d "${loop_runtime_root}/.tools/protocol-generation.XXXXXX")"
cleanup() {
  rm -rf -- "${loop_staging_dir}"
}
trap cleanup EXIT

loop_descriptor="${loop_staging_dir}/schema.binpb"
loop_ts_staging="${loop_staging_dir}/packages/protocol-ts/src/generated"
loop_rust_staging="${loop_staging_dir}/crates/loop-protocol/src/generated"
loop_python_staging="${loop_staging_dir}/python/loop_protocol/src/loop"

cd "${loop_repo_dir}"
./scripts/pnpm.sh exec buf format --diff --exit-code
./scripts/pnpm.sh exec buf lint
./scripts/pnpm.sh exec buf build --as-file-descriptor-set --exclude-source-info \
  --output "${loop_descriptor}"
./scripts/pnpm.sh exec buf generate --template buf.gen.yaml --output "${loop_staging_dir}"

./scripts/cargo.sh run --locked --offline --quiet -p loop-protocol-codegen -- \
  "${loop_descriptor}" "${loop_rust_staging}"

loop_grpc_include="$({
  ./scripts/uv-protocol.sh run --locked --offline python -c \
    'from pathlib import Path; import grpc_tools; print(Path(grpc_tools.__file__).parent / "_proto")'
})"
mapfile -d '' loop_proto_files < <(
  find "${loop_repo_dir}/proto" -type f -name '*.proto' -print0 | sort -z
)
if (( ${#loop_proto_files[@]} == 0 )); then
  echo "no Protobuf sources found under ${loop_repo_dir}/proto" >&2
  exit 2
fi

mkdir -p "${loop_staging_dir}/python/loop_protocol/src"
./scripts/uv-protocol.sh run --locked --offline python -m grpc_tools.protoc \
  --proto_path="${loop_repo_dir}/proto" \
  --proto_path="${loop_grpc_include}" \
  --python_out="${loop_staging_dir}/python/loop_protocol/src" \
  --pyi_out="${loop_staging_dir}/python/loop_protocol/src" \
  --grpc_python_out="${loop_staging_dir}/python/loop_protocol/src" \
  "${loop_proto_files[@]}"

while IFS= read -r -d '' loop_python_package; do
  install -m 0644 /dev/null "${loop_python_package}/__init__.py"
done < <(find "${loop_python_staging}" -type d -print0)
install -m 0644 /dev/null "${loop_python_staging}/py.typed"

loop_ts_target="${loop_repo_dir}/packages/protocol-ts/src/generated"
loop_rust_target="${loop_repo_dir}/crates/loop-protocol/src/generated"
loop_python_target="${loop_repo_dir}/python/loop_protocol/src/loop"
loop_descriptor_target="${loop_repo_dir}/fixtures/contracts/protocol/v1/schema.current.binpb"
loop_baseline_target="${loop_repo_dir}/fixtures/contracts/protocol/v1/schema.baseline.binpb"

if [[ "${loop_mode}" == "--check" ]]; then
  diff -ruN "${loop_ts_target}" "${loop_ts_staging}"
  diff -ruN "${loop_rust_target}" "${loop_rust_staging}"
  diff -ruN --exclude='__pycache__' --exclude='*.pyc' \
    "${loop_python_target}" "${loop_python_staging}"
  cmp "${loop_descriptor_target}" "${loop_descriptor}"
  echo "Protocol sources and committed generated artifacts match."
  exit 0
fi

if [[ "${loop_mode}" == "--initialize-baseline" && -e "${loop_baseline_target}" ]]; then
  echo "refusing to overwrite compatibility baseline: ${loop_baseline_target}" >&2
  exit 2
fi

if [[ "${loop_mode}" == "--write" && ! -f "${loop_baseline_target}" ]]; then
  echo "compatibility baseline is absent; use --initialize-baseline once" >&2
  exit 2
fi

replace_generated_tree() {
  local loop_source="$1"
  local loop_target="$2"

  mkdir -p "$(dirname -- "${loop_target}")"
  rm -rf -- "${loop_target}"
  cp -a "${loop_source}" "${loop_target}"
}

replace_generated_tree "${loop_ts_staging}" "${loop_ts_target}"
replace_generated_tree "${loop_rust_staging}" "${loop_rust_target}"
replace_generated_tree "${loop_python_staging}" "${loop_python_target}"
mkdir -p "$(dirname -- "${loop_descriptor_target}")"
install -m 0644 "${loop_descriptor}" "${loop_descriptor_target}"

if [[ "${loop_mode}" == "--initialize-baseline" ]]; then
  install -m 0444 "${loop_descriptor}" "${loop_baseline_target}"
fi

echo "Protocol bindings and current descriptor were regenerated."
