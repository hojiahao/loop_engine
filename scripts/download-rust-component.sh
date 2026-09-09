#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: download-rust-component.sh <mirror> <dist-path> <sha256> <output>" >&2
  exit 2
fi

loop_primary="$1"
loop_dist_path="$2"
loop_digest="$3"
loop_output="$4"
case "${loop_primary}" in
  sjtug|ustc|rsproxy|official) ;;
  *) echo "unsupported Rust distribution mirror" >&2; exit 2 ;;
esac
if [[ ! "${loop_digest}" =~ ^[0-9a-f]{64}$ ]] ||
  [[ ! "${loop_dist_path}" =~ ^dist/[0-9]{4}-[0-9]{2}-[0-9]{2}/(cargo|clippy|rust-std|rustc|rustfmt)-[0-9]+\.[0-9]+\.[0-9]+-x86_64-unknown-linux-gnu\.tar\.xz$ ]] ||
  [[ -z "${loop_output}" || -d "${loop_output}" || -L "${loop_output}" ]]; then
  echo "invalid Rust component download arguments" >&2
  exit 2
fi

loop_verify() {
  printf '%s  %s\n' "${loop_digest}" "$1" | sha256sum --check --status
}
if [[ -f "${loop_output}" ]] && loop_verify "${loop_output}"; then
  exit 0
fi

loop_partial="$(mktemp "${loop_output}.part.XXXXXX")"
loop_cleanup() {
  if [[ -n "${loop_partial}" ]]; then
    rm -f -- "${loop_partial}"
  fi
}
trap loop_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
declare -A loop_seen=()

# Transport failures may fall back; mismatched bytes never reach an installer.
for loop_mirror in "${loop_primary}" official rsproxy; do
  if [[ -n "${loop_seen[${loop_mirror}]:-}" ]]; then
    continue
  fi
  loop_seen["${loop_mirror}"]=1
  case "${loop_mirror}" in
    sjtug) loop_server="https://mirrors.sjtug.sjtu.edu.cn/rust-static" ;;
    ustc) loop_server="https://mirrors.ustc.edu.cn/rust-static" ;;
    rsproxy) loop_server="https://rsproxy.cn" ;;
    official) loop_server="https://static.rust-lang.org" ;;
  esac
  printf 'Rust component source: %s\n' "${loop_mirror}" >&2
  if curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --fail --silent --show-error --location \
    --connect-timeout 10 --max-time 300 \
    "${loop_server}/${loop_dist_path}" --output "${loop_partial}"; then
    if ! loop_verify "${loop_partial}"; then
      echo "Rust component checksum mismatch; refusing installation" >&2
      exit 1
    fi
    mv -fT -- "${loop_partial}" "${loop_output}"
    loop_partial=""
    exit 0
  fi
  printf 'Rust component transport failed: %s\n' "${loop_mirror}" >&2
done
echo "All approved Rust component sources failed" >&2
exit 1
