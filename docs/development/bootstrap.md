# Reproducible development environment

## Pinned versions

| Tool | Version | Source of truth |
| --- | ---: | --- |
| Rust / Cargo | 1.93.1 | `rust-toolchain.toml` |
| Node.js | 24.17.0 | root `package.json` and development container |
| pnpm | 11.25.0 | root `package.json` |
| Python | CPython 3.14.4 | root `.python-version` and development container |
| uv | 0.11.29 | CI and development container |
| just | 1.45.0 | development container and bootstrap script |

## Container image policy

Container images are pulled through the DaoCloud prefix. For example, the
upstream `docker.io/library/python` image is referenced as
`m.daocloud.io/docker.io/library/python`. Every base image in the Dockerfile is
also pinned to a full OCI index digest.

The development image installs Cargo, rustc, rust-std, Clippy, and rustfmt from
the SJTUG copies of the official component archives. Before extraction, every
archive must match the SHA-256 manifest committed under `config/toolchains`.
This makes the mirror a transport rather than a provenance authority. A clean
runtime volume does not download a second copy of Rust or pull the large Rust
OCI image layer.

Debian packages inside the image use the official `https://deb.debian.org`
endpoints, including the security repository. This APT transport is independent
of the DaoCloud OCI transport. Debian signatures and Release-file expiration
checks remain mandatory. Package requests have bounded retries and timeouts;
any failed index fetch aborts the build instead of accepting stale cached lists.
The previous Tencent Cloud mirror served expired security metadata in CI run
`34181369455`; bypassing expiration checks is not an accepted workaround.

Run the full clean-container gate with:

```bash
just container-gate
```

`container-gate` creates a unique Compose project name, starts with new
`development-python` and `development-runtime` volumes, runs bootstrap plus all
repository gates, and removes those containers and volumes on both success and
failure. It does not reuse or delete an interactive developer's default Compose
volumes.

The Docker daemon does not need a global registry-mirror mutation. Keeping the
mirror prefix in source makes the effective transport reviewable per image.
The explicit UID/GID mapping prevents bind-mounted source files from changing
ownership. A named `development-runtime` volume holds container-only tool and
download caches. A second named `development-python` volume is mounted at the
conventional `/workspace/.venv` path, so the container has one root project
environment without reusing the host environment or its absolute interpreter
paths. Both targets use sticky temporary-directory permissions so the
explicitly mapped caller UID can initialize them.

The development image is pinned by digest, but its Debian packages are resolved
from the package sources configured in that image. This phase guarantees pinned
language tools, lock-consistent dependencies, and a known base image; it does
not claim byte-identical container rebuilds. Production package snapshots and
SBOM verification are a Phase 14 release gate.

## Host bootstrap

The supported host path is Linux x86-64 with Node.js 24.17.0, Corepack, uv
0.11.29, curl, SHA-256 tooling, tar, and xz available. `$HOME/.local/bin` must
be on `PATH` when just is not already installed. Start with the script because
a clean host may not have just yet:

```bash
./scripts/bootstrap.sh
just check
just test
just build
just doctor
```

Bootstrap reuses a Rust installation only when all four tool versions match,
its resolved sysroot contains the component checksum marker, and that marker
exactly matches the repository manifest. Ordinary system packages and rustup
toolchains have no such provenance marker and are not reused. Bootstrap instead
downloads the five component archives into a temporary directory on the
runtime filesystem, verifies each archive against
`config/toolchains/rust-1.93.1-x86_64-unknown-linux-gnu.sha256`, and installs
them atomically under the ignored runtime `.tools` directory.
`LOOP_ENGINE_RUST_DIST_MIRROR` selects a reviewed transport and defaults to
`sjtug`; accepted values are `rsproxy`, `ustc`, `sjtug`, and `official`.
Unknown values fail closed instead of turning bootstrap into an arbitrary
downloader. All redirects must remain HTTPS and all endpoints are subject to
the same pinned digests. This selection affects Rust archives only, not OCI
image pulls.
The root
`pyproject.toml` defines one uv workspace for the legacy regression dependencies,
`loop_research`, and `loop_protocol`. Bootstrap creates one Git-ignored `.venv`
at the repository root with standard CPython 3.14.4 and synchronizes all
workspace packages and dependency groups from the single root `uv.lock`. uv download and managed
interpreter caches remain under runtime `.tools`; a `LOOP_ENGINE_RUNTIME_ROOT`
override moves those caches but never moves the project `.venv`. If just is
absent, bootstrap verifies the pinned release archive in `.tools`, installs the
binary into `$HOME/.local/bin`, and removes the archive. The archive is fetched
through DaoCloud's GitHub binary mirror and checked against the
repository-pinned upstream SHA-256.

Cargo downloads use the repository-scoped RSProxy sparse index configuration in
`.cargo/config.toml`. Package identity and integrity remain fixed by
`Cargo.lock`; no global Cargo configuration is modified. Bootstrap fills the
repository-local Cargo cache once. Check, test, build, and doctor commands then
run with locked and offline dependency resolution so verification cannot drift
with network state. Bootstrap uses `--locked`, rather than `--frozen`, so a
manifest/lock mismatch fails immediately.

The command wrappers honor `LOOP_ENGINE_RUNTIME_ROOT` for isolated container or
CI cache state. They switch to a runtime-local direct Rust installation only
when its Cargo executable exists; otherwise a pinned system or container
toolchain remains authoritative. pnpm uses an explicit store under
that runtime root and copy import mode, so a container volume never redirects
its content store into the bind-mounted checkout.

Use `./scripts/uv.sh` for workspace-wide Python commands and every environment
or dependency mutation. The package wrappers `uv-research.sh` and
`uv-protocol.sh` accept only `run`, select their owning workspace package, and
change to that package's working directory; all three wrappers resolve the same
root lock and `.venv`. CI additionally runs each package's tests through uv's
ephemeral isolated mode so undeclared workspace imports fail. The environment
gate rejects stale `.venv-*` directories, package-local virtual environments,
non-CPython interpreters, and free-threaded CPython builds.

Every repository Shell entry point is parsed by `bash -n` during `just check`.

Reviewed Rust distribution mirror configuration references:

- [RSProxy](https://rsproxy.cn/)
- [USTC](https://mirrors.ustc.edu.cn/help/rust-static.html)
- [SJTUG](https://mirrors.sjtug.sjtu.edu.cn/docs/rust-static)

Tsinghua TUNA documents Rust distribution support but does not currently retain the
pinned Rust 1.93.1 channel manifest, so it is not an accepted selector for this
toolchain revision.

No bootstrap command reads production credentials. Provider and market-data
secrets are introduced only by later runtime secret references.
