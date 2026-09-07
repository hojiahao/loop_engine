#!/usr/bin/env bash

# Sourced by bootstrap.sh and cargo.sh. Callers own shell options and exports.
loop_rust_metadata_init() {
  local repo_dir="$1"
  local runtime_root="$2"

  loop_rust_target="x86_64-unknown-linux-gnu"
  loop_rust_version="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "${repo_dir}/rust-toolchain.toml")"
  loop_rust_checksums="${repo_dir}/config/toolchains/rust-${loop_rust_version}-${loop_rust_target}.sha256"
  loop_rust_local_prefix="${runtime_root}/.tools/rust-${loop_rust_version}-${loop_rust_target}"
  loop_expected_rustc="rustc 1.93.1 (01f6ddf75 2026-02-11)"
  loop_expected_cargo="cargo 1.93.1 (083ac5135 2025-12-15)"
  loop_expected_clippy="clippy 0.1.93 (01f6ddf758 2026-02-11)"
  loop_expected_rustfmt="rustfmt 1.8.0-stable (01f6ddf758 2026-02-11)"
}

loop_rust_prefix_is_valid() {
  local prefix="$1"
  local resolved_prefix
  local resolved_sysroot

  [[ -x "${prefix}/bin/rustc" ]] &&
    [[ -x "${prefix}/bin/cargo" ]] &&
    [[ -x "${prefix}/bin/cargo-clippy" ]] &&
    [[ -x "${prefix}/bin/rustfmt" ]] &&
    [[ -f "${prefix}/.loop-engine-components.sha256" ]] || return 1
  [[ "$("${prefix}/bin/rustc" --version)" == "${loop_expected_rustc}" ]] || return 1
  [[ "$("${prefix}/bin/cargo" --version)" == "${loop_expected_cargo}" ]] || return 1
  [[ "$("${prefix}/bin/cargo-clippy" --version)" == "${loop_expected_clippy}" ]] || return 1
  [[ "$("${prefix}/bin/rustfmt" --version)" == "${loop_expected_rustfmt}" ]] || return 1
  cmp --silent "${loop_rust_checksums}" "${prefix}/.loop-engine-components.sha256" || return 1
  resolved_prefix="$(realpath -e -- "${prefix}")" || return 1
  resolved_sysroot="$(realpath -e -- "$("${prefix}/bin/rustc" --print sysroot)")" || return 1
  [[ "${resolved_prefix}" == "${resolved_sysroot}" ]]
}

loop_system_rust_prefix() {
  command -v rustc >/dev/null 2>&1 || return 1
  rustc --print sysroot 2>/dev/null
}
