#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_expected_version="$(tr -d '[:space:]' < "${loop_repo_dir}/.python-version")"
loop_expected_prefix="$(realpath "${loop_repo_dir}/.venv")"

loop_environment_record="$({
  "${loop_repo_dir}/scripts/uv.sh" run --all-packages --all-groups \
    --locked --offline --no-sync python -c \
    'import platform, sys, sysconfig; print(platform.python_version()); print(platform.python_implementation()); print(int(sysconfig.get_config_var("Py_GIL_DISABLED") or 0)); print(sys.prefix)'
})"
mapfile -t loop_environment_lines <<< "${loop_environment_record}"

if [[ "${loop_environment_lines[0]:-}" != "${loop_expected_version}" ]]; then
  echo "Python ${loop_expected_version} is required; found ${loop_environment_lines[0]:-missing}" >&2
  exit 2
fi

if [[ "${loop_environment_lines[1]:-}" != "CPython" ]]; then
  echo "CPython is required; found ${loop_environment_lines[1]:-missing}" >&2
  exit 2
fi

if [[ "${loop_environment_lines[2]:-}" != "0" ]]; then
  echo "the standard GIL-enabled CPython build is required" >&2
  exit 2
fi

if [[ "$(realpath "${loop_environment_lines[3]:-missing}")" != "${loop_expected_prefix}" ]]; then
  echo "uv must use the repository root environment: ${loop_expected_prefix}" >&2
  exit 2
fi

while IFS= read -r loop_unexpected_environment; do
  if [[ "$(realpath "${loop_unexpected_environment}")" != "${loop_expected_prefix}" ]]; then
    echo "unexpected project virtual environment: ${loop_unexpected_environment}" >&2
    exit 2
  fi
done < <(
  find "${loop_repo_dir}" \
    \( -path "${loop_repo_dir}/.git" -o -path "${loop_repo_dir}/.tools" -o \
       -path "${loop_repo_dir}/node_modules" \) -prune -o \
    -type d -name '.venv*' -print
)

if [[ "$(realpath -m "${loop_runtime_root}")" != "$(realpath "${loop_repo_dir}")" ]]; then
  while IFS= read -r loop_stale_environment; do
    echo "stale pre-workspace virtual environment: ${loop_stale_environment}" >&2
    exit 2
  done < <(
    find "${loop_runtime_root}" -maxdepth 2 -type d \
      \( -name '.venv-legacy' -o -name '.venv-research' -o \
         -name '.venv-protocol' \) -print
  )
fi

echo "CPython ${loop_expected_version} uses the single root .venv."
