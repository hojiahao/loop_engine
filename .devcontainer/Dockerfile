ARG PYTHON_IMAGE="m.daocloud.io/docker.io/library/python:3.14.4-slim-bookworm@sha256:fc74d22ffd0d5ac395a4b7bdda75a4539758862c49ebf3005647084631e63789"
ARG NODE_IMAGE="m.daocloud.io/docker.io/library/node:24.17.0-bookworm-slim@sha256:862263c612aa437e3037674b85419622a9d93bff80aa1eee5398dfe686375532"
ARG UV_IMAGE="m.daocloud.io/ghcr.io/astral-sh/uv:0.11.29@sha256:eb2843a1e56fd9e30c7276ce1a52cba86e64c7b385f5e3279a0e08e02dd058fc"
ARG RUST_VERSION="1.93.1"
ARG RUST_RELEASE_DATE="2026-02-12"
ARG RUST_TARGET="x86_64-unknown-linux-gnu"
ARG JUST_VERSION="1.45.0"
ARG JUST_SHA256="dc3f958aaf8c6506dd90426e9b03f86dd15e74a6467ee0e54929f750af3d9e49"

FROM ${NODE_IMAGE} AS node_toolchain
FROM ${UV_IMAGE} AS uv_toolchain
FROM ${PYTHON_IMAGE} AS development

ARG RUST_VERSION
ARG RUST_RELEASE_DATE
ARG RUST_TARGET
ARG JUST_VERSION
ARG JUST_SHA256

ENV RUST_PREFIX=/usr/local/rust \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/rust/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_REGISTRIES_CRATES_IO_INDEX=sparse+https://rsproxy.cn/index/ \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
    CARGO_HTTP_TIMEOUT=30 \
    CARGO_NET_RETRY=3 \
    PNPM_HOME=/usr/local/share/pnpm \
    COREPACK_HOME=/usr/local/share/corepack \
    UV_LINK_MODE=copy \
    UV_PYTHON_DOWNLOADS=never

COPY --from=node_toolchain /usr/local/bin/node /usr/local/bin/node
COPY --from=node_toolchain /usr/local/lib/node_modules /usr/local/lib/node_modules
COPY --from=uv_toolchain /uv /uvx /usr/local/bin/

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    sed -i 's|http://deb.debian.org|https://mirrors.cloud.tencent.com|g' \
        /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Retries=3 -o Acquire::https::Timeout=30 update \
    && DEBIAN_FRONTEND=noninteractive apt-get \
        -o Acquire::Retries=3 -o Acquire::https::Timeout=30 \
        install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        libsqlite3-dev \
        libssl-dev \
        pkg-config \
        protobuf-compiler \
        sqlite3 \
        xz-utils

COPY config/toolchains/rust-1.93.1-x86_64-unknown-linux-gnu.sha256 /tmp/rust-components.sha256

RUN --mount=type=cache,id=loop-engine-rust-1.93.1,target=/var/cache/loop-rust,sharing=locked \
    set -eu; \
    test "$(wc -l < /tmp/rust-components.sha256)" -eq 5; \
    while read -r digest archive; do \
        case "${archive}" in \
            cargo-${RUST_VERSION}-${RUST_TARGET}.tar.xz|\
            clippy-${RUST_VERSION}-${RUST_TARGET}.tar.xz|\
            rust-std-${RUST_VERSION}-${RUST_TARGET}.tar.xz|\
            rustc-${RUST_VERSION}-${RUST_TARGET}.tar.xz|\
            rustfmt-${RUST_VERSION}-${RUST_TARGET}.tar.xz) ;; \
            *) echo "unexpected Rust component archive: ${archive}" >&2; exit 2 ;; \
        esac; \
        archive_path="/var/cache/loop-rust/${archive}"; \
        if [ ! -f "${archive_path}" ] || \
            ! echo "${digest}  ${archive_path}" | sha256sum --check --status; then \
            rm -f -- "${archive_path}.part"; \
            curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
                --fail --silent --show-error --location --retry 3 \
                --connect-timeout 15 --max-time 600 \
                "https://mirrors.sjtug.sjtu.edu.cn/rust-static/dist/${RUST_RELEASE_DATE}/${archive}" \
                --output "${archive_path}.part"; \
            echo "${digest}  ${archive_path}.part" | sha256sum --check; \
            mv "${archive_path}.part" "${archive_path}"; \
        fi; \
        echo "${digest}  ${archive_path}" | sha256sum --check; \
    done < /tmp/rust-components.sha256

RUN --network=none \
    --mount=type=cache,id=loop-engine-rust-1.93.1,target=/var/cache/loop-rust,sharing=locked,readonly \
    set -eu; \
    install_root="$(mktemp -d)"; \
    while read -r digest archive; do \
        echo "${digest}  /var/cache/loop-rust/${archive}" | sha256sum --check; \
        component_dir="${install_root}/${archive%.tar.xz}"; \
        tar -xJf "/var/cache/loop-rust/${archive}" -C "${install_root}"; \
        "${component_dir}/install.sh" --prefix="${RUST_PREFIX}" --disable-ldconfig; \
        rm -rf -- "${component_dir}"; \
    done < /tmp/rust-components.sha256; \
    rmdir "${install_root}"; \
    cp /tmp/rust-components.sha256 "${RUST_PREFIX}/.loop-engine-components.sha256"; \
    rm /tmp/rust-components.sha256; \
    test "$(rustc --version)" = "rustc 1.93.1 (01f6ddf75 2026-02-11)"; \
    test "$(cargo --version)" = "cargo 1.93.1 (083ac5135 2025-12-15)"; \
    test "$(cargo-clippy --version)" = "clippy 0.1.93 (01f6ddf758 2026-02-11)"; \
    test "$(rustfmt --version)" = "rustfmt 1.8.0-stable (01f6ddf758 2026-02-11)"; \
    test "$(rustc --print sysroot)" = "${RUST_PREFIX}"

RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && ln -s /usr/local/lib/node_modules/corepack/dist/corepack.js /usr/local/bin/corepack \
    && corepack install --global pnpm@11.25.0

RUN curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --fail --silent --show-error --location \
        "https://files.m.daocloud.io/github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
        --output /tmp/just.tar.gz \
    && echo "${JUST_SHA256}  /tmp/just.tar.gz" | sha256sum --check \
    && tar -xzf /tmp/just.tar.gz -C /usr/local/bin just \
    && rm /tmp/just.tar.gz \
    && test "$(just --version)" = "just ${JUST_VERSION}"

RUN useradd --create-home --shell /bin/bash --uid 1000 loop \
    && install --directory --mode 1777 /var/lib/loop-engine-dev \
    && install --directory --mode 1777 /workspace/.venv

USER loop
WORKDIR /workspace
CMD ["sleep", "infinity"]
