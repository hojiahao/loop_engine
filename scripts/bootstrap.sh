#!/usr/bin/env bash
set -euo pipefail

loop_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
loop_runtime_root="${LOOP_ENGINE_RUNTIME_ROOT:-${loop_repo_dir}}"
loop_tools_dir="${loop_runtime_root}/.tools"
loop_rustup_home="${loop_tools_dir}/rustup"
loop_cargo_home="${loop_tools_dir}/cargo"
loop_rustup_target="x86_64-unknown-linux-gnu"
loop_rustup_init_sha256="dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71"
loop_rust_version="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${loop_repo_dir}/rust-toolchain.toml")"
loop_node_version="24.17.0"
loop_pnpm_version="11.25.0"
loop_python_version="3.12.13"
loop_uv_version="0.11.29"
loop_just_version="1.45.0"
loop_just_sha256="dc3f958aaf8c6506dd90426e9b03f86dd15e74a6467ee0e54929f750af3d9e49"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "bootstrap currently supports Linux x86_64 only" >&2
  exit 2
fi

if [[ -z "${loop_rust_version}" ]]; then
  echo "rust-toolchain.toml does not declare a channel" >&2
  exit 2
fi

for loop_command in curl sha256sum node corepack uv; do
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

mkdir -p "${loop_tools_dir}" "${loop_cargo_home}" "${loop_rustup_home}"

if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1 && \
  command -v rustfmt >/dev/null 2>&1 && command -v cargo-clippy >/dev/null 2>&1 && \
  [[ "$(rustc --version | awk '{print $2}')" == "${loop_rust_version}" ]] && \
  [[ "$(cargo --version | awk '{print $2}')" == "${loop_rust_version}" ]]; then
  loop_use_system_rust=1
else
  if [[ ! -x "${loop_cargo_home}/bin/rustup" ]]; then
    loop_download_dir="$(mktemp -d)"
    trap 'rm -rf "${loop_download_dir}"' EXIT
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
      --fail --silent --show-error --location \
    "https://static.rust-lang.org/rustup/dist/${loop_rustup_target}/rustup-init" \
      --output "${loop_download_dir}/rustup-init"
    echo "${loop_rustup_init_sha256}  ${loop_download_dir}/rustup-init" | sha256sum --check
    chmod 0755 "${loop_download_dir}/rustup-init"
    CARGO_HOME="${loop_cargo_home}" RUSTUP_HOME="${loop_rustup_home}" \
      "${loop_download_dir}/rustup-init" -y --no-modify-path --profile minimal \
      --default-toolchain "${loop_rust_version}" --component clippy --component rustfmt
  else
    CARGO_HOME="${loop_cargo_home}" RUSTUP_HOME="${loop_rustup_home}" \
      "${loop_cargo_home}/bin/rustup" toolchain install "${loop_rust_version}" \
      --profile minimal --component clippy --component rustfmt
  fi
fi

export CARGO_HOME="${loop_cargo_home}"
export PATH="${CARGO_HOME}/bin:${PATH}"

if [[ "${loop_use_system_rust:-0}" != "1" ]]; then
  export RUSTUP_HOME="${loop_rustup_home}"
fi
export COREPACK_HOME="${COREPACK_HOME:-${loop_tools_dir}/corepack}"
export UV_CACHE_DIR="${loop_tools_dir}/uv-cache"
export UV_PYTHON_INSTALL_DIR="${loop_tools_dir}/python"

if [[ "$(rustc --version | awk '{print $2}')" != "${loop_rust_version}" ]] || \
  [[ "$(cargo --version | awk '{print $2}')" != "${loop_rust_version}" ]] || \
  ! command -v rustfmt >/dev/null 2>&1 || ! command -v cargo-clippy >/dev/null 2>&1; then
  echo "Rust ${loop_rust_version} with rustfmt and clippy is required" >&2
  exit 2
fi

if [[ "$(corepack pnpm --version)" != "${loop_pnpm_version}" ]]; then
  echo "pnpm ${loop_pnpm_version} could not be activated through Corepack" >&2
  exit 2
fi

if command -v python3 >/dev/null 2>&1 && \
  [[ "$(python3 -c 'import platform; print(platform.python_version())')" == "${loop_python_version}" ]]; then
  loop_python="$(command -v python3)"
else
  uv python install "${loop_python_version}"
  loop_python="$(uv python find "${loop_python_version}")"
fi

UV_PROJECT_ENVIRONMENT="${loop_runtime_root}/.venv-legacy" \
  uv sync --project "${loop_repo_dir}" --python "${loop_python}" --locked
UV_PROJECT_ENVIRONMENT="${loop_runtime_root}/.venv-research" \
  uv sync --project "${loop_repo_dir}/python/loop_research" --python "${loop_python}" --locked

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

printf 'rustc %s\nnode %s\npnpm %s\npython %s\nuv %s\njust %s\n' \
  "${loop_rust_version}" "${loop_node_version}" "${loop_pnpm_version}" \
  "${loop_python_version}" "${loop_uv_version}" "${loop_just_version}"
echo "Loop Engine toolchain is ready."
