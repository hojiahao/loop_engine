#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_baseline="${loop_repo_dir}/fixtures/contracts/protocol/v1/schema.baseline.binpb"
loop_history_dir="${loop_repo_dir}/fixtures/contracts/protocol/history"
loop_expected_sha256="27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979"
loop_expected_history_sha256s=(
  "041a396de9b59c4ceb3c40f160f7e2ddda404f362ea72ab6929e1314eed510d1"
  "4cd9d0e49e174f3a2920bbee19921f4f48b459aa48ac6a5927efa48722ff81f7"
  "51a4c938810cede021a2cc2682654a52fb811efc8a16fa7d9c2ced88d00529f1"
  "62b560d6df574b152aa5e6cdcbad46834da3ecb0a923ced6aaa47c0fd91246f3"
  "fdbb927352b591c85a1ed054f3200a0234e98db643f79cd9ef9e6f30d7868563"
)
loop_mode="${1:---check}"

if [[ "${loop_mode}" != "--check" && "${loop_mode}" != "--self-test" ]]; then
  echo "usage: $0 [--check|--self-test]" >&2
  exit 2
fi

sha256_file() {
  local loop_path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${loop_path}" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${loop_path}" | awk '{print $1}'
    return
  fi
  echo "neither sha256sum nor shasum is available" >&2
  return 2
}

verify_baseline() {
  local loop_path="$1"
  if [[ ! -f "${loop_path}" ]]; then
    echo "protocol compatibility baseline is missing: ${loop_path}" >&2
    return 1
  fi

  local loop_actual_sha256
  loop_actual_sha256="$(sha256_file "${loop_path}")"
  if [[ "${loop_actual_sha256}" != "${loop_expected_sha256}" ]]; then
    echo "protocol compatibility baseline digest mismatch: expected ${loop_expected_sha256}, found ${loop_actual_sha256}" >&2
    return 1
  fi
}

verify_history() {
  local loop_directory="$1"
  if [[ ! -d "${loop_directory}" ]]; then
    echo "protocol descriptor history is missing: ${loop_directory}" >&2
    return 1
  fi

  local loop_expected_history_sha256
  for loop_expected_history_sha256 in "${loop_expected_history_sha256s[@]}"; do
    if [[ ! -f "${loop_directory}/${loop_expected_history_sha256}.binpb" ]]; then
      echo "protocol descriptor history fixture is missing: ${loop_expected_history_sha256}.binpb" >&2
      return 1
    fi
  done

  local loop_found=0
  local loop_path
  while IFS= read -r -d '' loop_path; do
    ((loop_found += 1))
    local loop_expected_history_sha256
    local loop_actual_history_sha256
    loop_expected_history_sha256="$(basename -- "${loop_path}" .binpb)"
    local loop_known=false
    local loop_known_sha256
    for loop_known_sha256 in "${loop_expected_history_sha256s[@]}"; do
      if [[ "${loop_expected_history_sha256}" == "${loop_known_sha256}" ]]; then
        loop_known=true
        break
      fi
    done
    if [[ "${loop_known}" != true ]]; then
      echo "unreviewed protocol history fixture: ${loop_path}" >&2
      return 1
    fi
    loop_actual_history_sha256="$(sha256_file "${loop_path}")"
    if [[ "${loop_actual_history_sha256}" != "${loop_expected_history_sha256}" ]]; then
      echo "protocol history digest mismatch: ${loop_path} names ${loop_expected_history_sha256}, found ${loop_actual_history_sha256}" >&2
      return 1
    fi
  done < <(find "${loop_directory}" -maxdepth 1 -type f -name '*.binpb' -print0)
  if (( loop_found != ${#loop_expected_history_sha256s[@]} )); then
    echo "protocol descriptor history fixture count mismatch" >&2
    return 1
  fi
}

verify_baseline "${loop_baseline}"
verify_history "${loop_history_dir}"

if [[ "${loop_mode}" == "--self-test" ]]; then
  mkdir -p "${loop_runtime_root}/.tools"
  loop_test_dir="$(mktemp -d "${loop_runtime_root}/.tools/protocol-baseline-guard.XXXXXX")"
  cleanup() {
    rm -rf -- "${loop_test_dir}"
  }
  trap cleanup EXIT

  loop_tampered_baseline="${loop_test_dir}/schema.baseline.binpb"
  cp -- "${loop_baseline}" "${loop_tampered_baseline}"
  chmod u+w "${loop_tampered_baseline}"
  printf '\0' >>"${loop_tampered_baseline}"
  if verify_baseline "${loop_tampered_baseline}" >/dev/null 2>&1; then
    echo "protocol baseline guard accepted a tampered temporary copy" >&2
    exit 1
  fi

  loop_tampered_history="${loop_test_dir}/history"
  cp -a -- "${loop_history_dir}" "${loop_tampered_history}"
  chmod u+w "${loop_tampered_history}/${loop_expected_history_sha256s[0]}.binpb"
  printf '\0' >>"${loop_tampered_history}/${loop_expected_history_sha256s[0]}.binpb"
  if verify_history "${loop_tampered_history}" >/dev/null 2>&1; then
    echo "protocol history guard accepted a tampered temporary archive" >&2
    exit 1
  fi
  cp -- "${loop_history_dir}/${loop_expected_history_sha256s[0]}.binpb" \
    "${loop_tampered_history}/${loop_expected_history_sha256s[0]}.binpb"
  rm -- "${loop_tampered_history}/${loop_expected_history_sha256s[1]}.binpb"
  if verify_history "${loop_tampered_history}" >/dev/null 2>&1; then
    echo "protocol history guard accepted an incomplete temporary archive" >&2
    exit 1
  fi
  echo "Protocol baseline and history inventory are pinned; tamper checks fail closed."
else
  echo "Protocol baseline digest matches the unreleased Phase 2 trust-anchor candidate."
fi
