#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_tools_dir="${loop_runtime_root}/.tools"
loop_cargo_home="${loop_tools_dir}/cargo"
loop_rust_release_date="2026-02-12"
loop_rust_dist_mirror="${LOOP_ENGINE_RUST_DIST_MIRROR:-sjtug}"
source "${loop_repo_dir}/scripts/rust-toolchain-common.sh"
loop_rust_metadata_init "${loop_repo_dir}" "${loop_runtime_root}"
loop_node_version="24.17.0"
loop_pnpm_version="11.25.0"
loop_python_version="3.14.4"
loop_uv_version="0.11.29"
loop_just_version="1.45.0"
loop_just_sha256="dc3f958aaf8c6506dd90426e9b03f86dd15e74a6467ee0e54929f750af3d9e49"
loop_python_identity_code='import platform, sysconfig; print("|".join((platform.python_implementation(), platform.python_version(), str(int(sysconfig.get_config_var("Py_GIL_DISABLED") or 0)))))'

case "${loop_rust_dist_mirror}" in
  rsproxy|ustc|sjtug|official) ;;
  *)
    echo "unsupported LOOP_ENGINE_RUST_DIST_MIRROR: ${loop_rust_dist_mirror}" >&2
    echo "expected one of: rsproxy, ustc, sjtug, official" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "bootstrap currently supports Linux x86_64 only" >&2
  exit 2
fi

if [[ -z "${loop_rust_version}" ]]; then
  echo "rust-toolchain.toml does not declare a channel" >&2
  exit 2
fi

if [[ ! -f "${loop_rust_checksums}" ]] ||
  [[ "$(wc -l < "${loop_rust_checksums}")" -ne 5 ]]; then
  echo "missing or invalid Rust component checksum manifest: ${loop_rust_checksums}" >&2
  exit 2
fi

for loop_command in cmp curl realpath sha256sum tar xz node corepack uv; do
  if ! command -v "${loop_command}" >/dev/null 2>&1; then
    echo "required bootstrap command is missing: ${loop_command}" >&2
    exit 2
  fi
done

if [[ "$(node --version)" != "v${loop_node_version}" ]]; then
  echo "Node.js ${loop_node_version} is required; found $(node --version)" >&2
  exit 2
fi

if [[ "$(uv --version | awk '{print $2}')" != "${loop_uv_version}" ]]; then
  echo "uv ${loop_uv_version} is required; found $(uv --version)" >&2
  exit 2
fi

mkdir -p "${loop_tools_dir}" "${loop_cargo_home}"

loop_system_prefix="$(loop_system_rust_prefix || true)"
if [[ -n "${loop_system_prefix}" ]] && loop_rust_prefix_is_valid "${loop_system_prefix}"; then
  loop_active_rust_prefix="${loop_system_prefix}"
elif loop_rust_prefix_is_valid "${loop_rust_local_prefix}"; then
  loop_active_rust_prefix="${loop_rust_local_prefix}"
else
  if [[ -e "${loop_rust_local_prefix}" ]]; then
    echo "existing local Rust prefix failed validation: ${loop_rust_local_prefix}" >&2
    exit 2
  fi

  loop_download_dir="$(mktemp -d "${loop_tools_dir}/.rust-download.XXXXXX")"
  loop_install_prefix="$(mktemp -d "${loop_tools_dir}/.rust-install.XXXXXX")"
  cleanup_rust_install() {
    [[ -z "${loop_download_dir}" ]] || rm -rf -- "${loop_download_dir}"
    [[ -z "${loop_install_prefix}" ]] || rm -rf -- "${loop_install_prefix}"
  }
  trap cleanup_rust_install EXIT

  while read -r loop_digest loop_archive; do
    case "${loop_archive}" in
      cargo-${loop_rust_version}-${loop_rust_target}.tar.xz|clippy-${loop_rust_version}-${loop_rust_target}.tar.xz|rust-std-${loop_rust_version}-${loop_rust_target}.tar.xz|rustc-${loop_rust_version}-${loop_rust_target}.tar.xz|rustfmt-${loop_rust_version}-${loop_rust_target}.tar.xz) ;;
      *)
        echo "unexpected Rust component archive: ${loop_archive}" >&2
        exit 2
        ;;
    esac
    bash "${loop_repo_dir}/scripts/download-rust-component.sh" \
      "${loop_rust_dist_mirror}" "dist/${loop_rust_release_date}/${loop_archive}" \
      "${loop_digest}" "${loop_download_dir}/${loop_archive}"
    echo "${loop_digest}  ${loop_download_dir}/${loop_archive}" | sha256sum --check
  done < "${loop_rust_checksums}"

  while read -r _ loop_archive; do
    loop_component_dir="${loop_download_dir}/${loop_archive%.tar.xz}"
    tar -xJf "${loop_download_dir}/${loop_archive}" -C "${loop_download_dir}"
    "${loop_component_dir}/install.sh" \
      --prefix="${loop_install_prefix}" --disable-ldconfig
  done < "${loop_rust_checksums}"

  cp "${loop_rust_checksums}" "${loop_install_prefix}/.loop-engine-components.sha256"
  if ! loop_rust_prefix_is_valid "${loop_install_prefix}"; then
    echo "installed Rust toolchain failed identity validation" >&2
    exit 2
  fi
  mv "${loop_install_prefix}" "${loop_rust_local_prefix}"
  loop_install_prefix=""
  if ! loop_rust_prefix_is_valid "${loop_rust_local_prefix}"; then
    echo "moved Rust toolchain failed identity validation" >&2
    exit 2
  fi
  loop_active_rust_prefix="${loop_rust_local_prefix}"
fi

export CARGO_HOME="${loop_cargo_home}"
export PATH="${loop_active_rust_prefix}/bin:${CARGO_HOME}/bin:${PATH}"
export COREPACK_HOME="${COREPACK_HOME:-${loop_tools_dir}/corepack}"
export UV_CACHE_DIR="${loop_tools_dir}/uv-cache"
export UV_PYTHON_INSTALL_DIR="${loop_tools_dir}/python"

if ! loop_rust_prefix_is_valid "${loop_active_rust_prefix}"; then
  echo "Rust ${loop_rust_version} component identity validation failed" >&2
  exit 2
fi

if [[ "$(corepack pnpm --version)" != "${loop_pnpm_version}" ]]; then
  echo "pnpm ${loop_pnpm_version} could not be activated through Corepack" >&2
  exit 2
fi

if command -v python3 >/dev/null 2>&1 && \
  [[ "$(python3 -c "${loop_python_identity_code}")" == \
  "CPython|${loop_python_version}|0" ]]; then
  loop_python="$(command -v python3)"
else
  uv python install "${loop_python_version}"
  loop_python="$(uv python find "${loop_python_version}")"
fi

"${loop_repo_dir}/scripts/uv.sh" sync --python "${loop_python}" \
  --all-packages --all-groups --locked
"${loop_repo_dir}/scripts/verify-python-environment.sh"

"${loop_repo_dir}/scripts/pnpm.sh" install --frozen-lockfile --config.confirmModulesPurge=false
cargo fetch --locked

if ! command -v just >/dev/null 2>&1; then
  loop_user_bin="${HOME}/.local/bin"
  if [[ ":${PATH}:" != *":${loop_user_bin}:"* ]]; then
    echo "${loop_user_bin} must be on PATH so bootstrap can install just" >&2
    exit 2
  fi
  mkdir -p "${loop_user_bin}"
  loop_just_archive="${loop_tools_dir}/just-${loop_just_version}.tar.gz"
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --fail --silent --show-error --location \
    "https://files.m.daocloud.io/github.com/casey/just/releases/download/${loop_just_version}/just-${loop_just_version}-x86_64-unknown-linux-musl.tar.gz" \
    --output "${loop_just_archive}"
  echo "${loop_just_sha256}  ${loop_just_archive}" | sha256sum --check
  tar -xzf "${loop_just_archive}" -C "${loop_user_bin}" just
  rm "${loop_just_archive}"
fi

if [[ "$(just --version)" != "just ${loop_just_version}" ]]; then
  echo "just ${loop_just_version} is required; found $(just --version)" >&2
  exit 2
fi

printf 'rustc %s\nrust distribution mirror %s\nnode %s\npnpm %s\npython %s\nuv %s\njust %s\n' \
  "${loop_rust_version}" "${loop_rust_dist_mirror}" "${loop_node_version}" \
  "${loop_pnpm_version}" \
  "${loop_python_version}" "${loop_uv_version}" "${loop_just_version}"
echo "Loop Engine toolchain is ready."
