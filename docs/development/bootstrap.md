# Reproducible development environment

## Pinned versions

| Tool | Version | Source of truth |
| --- | ---: | --- |
| Rust / Cargo | 1.93.1 | `rust-toolchain.toml` |
| Node.js | 24.17.0 | root `package.json` and development container |
| pnpm | 11.25.0 | root `package.json` |
| Python | 3.12.13 | research `.python-version` and development container |
| uv | 0.11.29 | CI and development container |
| just | 1.45.0 | development container and bootstrap script |

## Container image policy

Container images are pulled through the DaoCloud prefix. For example, the
upstream `docker.io/library/python` image is referenced as
`m.daocloud.io/docker.io/library/python`. Every base image in the Dockerfile is
also pinned to a full OCI index digest.

The Rust image layer includes the Clippy and rustfmt components for the exact
toolchain in `rust-toolchain.toml`. They are installed and version-checked while
the image is built, so a clean runtime volume does not download a second copy
of Rust.

Debian packages inside the image use the Tencent Cloud HTTPS mirror because direct
Debian HTTP transport is unreliable in the target network. Debian repository
signatures remain mandatory; the mirror is transport, not a trust authority.
Package requests have bounded retries and timeouts.

To start the environment:

```bash
export LOOP_ENGINE_UID="$(id -u)"
export LOOP_ENGINE_GID="$(id -g)"
docker compose build development
docker compose run --rm development just bootstrap
docker compose run --rm development just check
docker compose run --rm development just test
docker compose run --rm development just build
docker compose run --rm development just check
docker compose run --rm development just doctor
```

The Docker daemon does not need a global registry-mirror mutation. Keeping the
mirror prefix in source makes the effective transport reviewable per image.
The explicit UID/GID mapping prevents bind-mounted source files from changing
ownership. A named `development-runtime` volume holds container-only tool
caches and virtual environments, so absolute interpreter paths from a host
checkout can never leak into `/workspace`. The cache volume root uses sticky
temporary-directory permissions so the explicitly mapped caller UID can create
its own state without making existing entries mutable by other UIDs.

The development image is pinned by digest, but its Debian packages are resolved
from the package sources configured in that image. This phase guarantees pinned
language tools, lock-consistent dependencies, and a known base image; it does
not claim byte-identical container rebuilds. Production package snapshots and
SBOM verification are a Phase 14 release gate.

## Host bootstrap

The supported host path is Linux x86-64 with Node.js 24.17.0, Corepack, uv
0.11.29, curl, and SHA-256 tooling available. `$HOME/.local/bin` must be on
`PATH` when just is not already installed. Start with the script because a
clean host may not have just yet:

```bash
./scripts/bootstrap.sh
just check
just test
just build
just doctor
```

Bootstrap reuses a system Rust compiler only when it exactly matches
`rust-toolchain.toml`. Otherwise it downloads rustup into the repository-local
ignored `.tools` directory. The rustup bootstrap executable is restricted to
HTTPS redirects and verified against the SHA-256 value recorded in the script;
the requested Rust toolchain is then installed with rustfmt and Clippy. Python
environments and caches also remain repository-local. If just is absent,
bootstrap verifies the pinned release archive in `.tools`, installs the binary
into `$HOME/.local/bin`, and removes the archive. The archive is fetched through
DaoCloud's GitHub binary mirror and checked against the repository-pinned
upstream SHA-256.

Cargo downloads use the repository-scoped RSProxy sparse index configuration in
`.cargo/config.toml`. Package identity and integrity remain fixed by
`Cargo.lock`; no global Cargo configuration is modified. Bootstrap fills the
repository-local Cargo cache once. Check, test, build, and doctor commands then
run with locked and offline dependency resolution so verification cannot drift
with network state. Bootstrap uses `--locked`, rather than `--frozen`, so a
manifest/lock mismatch fails immediately.

The command wrappers honor `LOOP_ENGINE_RUNTIME_ROOT` for isolated container or
CI state. They switch to a repository-local rustup installation only when that
installation actually contains a toolchain; otherwise a pinned system or
container toolchain remains authoritative. pnpm uses an explicit store under
that runtime root and copy import mode, so a container volume never redirects
its content store into the bind-mounted checkout.

No bootstrap command reads production credentials. Provider and market-data
secrets are introduced only by later runtime secret references.
