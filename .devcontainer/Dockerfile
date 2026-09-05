ARG PYTHON_IMAGE="m.daocloud.io/docker.io/library/python:3.12.13-slim-bookworm@sha256:4766d8b510c428e595d74b9cc5bbb2fae8e26316fffb4adc89908d79aacd58a2"
ARG NODE_IMAGE="m.daocloud.io/docker.io/library/node:24.17.0-bookworm-slim@sha256:862263c612aa437e3037674b85419622a9d93bff80aa1eee5398dfe686375532"
ARG RUST_IMAGE="m.daocloud.io/docker.io/library/rust:1.93.1-slim-bookworm@sha256:5b9332190bb3b9ece73b810cd1f1e9f06343b294ce184bcb067f0747d7d333ea"
ARG UV_IMAGE="m.daocloud.io/ghcr.io/astral-sh/uv:0.11.29@sha256:eb2843a1e56fd9e30c7276ce1a52cba86e64c7b385f5e3279a0e08e02dd058fc"
ARG JUST_VERSION="1.45.0"
ARG JUST_SHA256="dc3f958aaf8c6506dd90426e9b03f86dd15e74a6467ee0e54929f750af3d9e49"

FROM ${NODE_IMAGE} AS node_toolchain
FROM ${RUST_IMAGE} AS rust_toolchain
FROM ${UV_IMAGE} AS uv_toolchain
FROM ${PYTHON_IMAGE} AS development

ARG JUST_VERSION
ARG JUST_SHA256

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
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
COPY --from=rust_toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust_toolchain /usr/local/rustup /usr/local/rustup
COPY --from=uv_toolchain /uv /uvx /usr/local/bin/

RUN RUSTUP_DIST_SERVER=https://rsproxy.cn \
    RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup \
    rustup component add --toolchain 1.93.1-x86_64-unknown-linux-gnu clippy rustfmt \
    && test "$(rustc --version | awk '{print $2}')" = "1.93.1" \
    && test "$(cargo --version | awk '{print $2}')" = "1.93.1" \
    && cargo clippy --version \
    && rustfmt --version

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
        sqlite3

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
    && install --directory --mode 1777 /var/lib/loop-engine-dev

USER loop
WORKDIR /workspace
CMD ["sleep", "infinity"]
