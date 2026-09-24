#!/usr/bin/env bash
set -euo pipefail

loop_output=""
loop_url=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --output) loop_output="$2"; shift 2 ;;
    https://*) loop_url="$1"; shift ;;
    *) shift ;;
  esac
done
[[ -n "${loop_output}" && -n "${loop_url}" ]]
printf '%s\n' "${loop_url}" >> "${LOOP_TEST_DOWNLOAD_LOG}"
case "${LOOP_TEST_DOWNLOAD_MODE}" in
  unavailable) printf 'partial' > "${loop_output}"; exit 56 ;;
  fallback)
    if [[ "${loop_url}" != https://static.rust-lang.org/* ]]; then
      printf 'partial' > "${loop_output}"; exit 28
    fi
    ;;
  secondary)
    if [[ "${loop_url}" != https://rsproxy.cn/* ]]; then
      exit 56
    fi
    ;;
  corrupt) printf 'corrupt bytes' > "${loop_output}"; exit 0 ;;
  interrupt) kill -TERM "${PPID}"; exit 56 ;;
  success) ;;
  *) exit 2 ;;
esac
printf 'pinned Rust component fixture' > "${loop_output}"
