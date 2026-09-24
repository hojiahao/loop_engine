# ADR 0002: Reproducible polyglot toolchain

- Status: Accepted
- Date: 2026-09-04
- Owners: hojiahao
- Python decision amended by: ADR 0005

## Context

Loop Engine spans a Rust control plane, a TypeScript provider/UI plane, and a
Python research plane. Floating language versions and unpinned container or CI
inputs would make financial results and operational failures difficult to
reproduce.

Direct Docker Hub access is not reliable in the deployment environment. The
operator requires DaoCloud as the image transport.

## Decision

1. Pin Rust 1.93.1, Node 24.17.0, pnpm 11.25.0, Python 3.12.13, uv 0.11.29,
   and just 1.45.0. ADR 0005 supersedes the Python pin and environment layout
   with Python 3.14.4 and one root uv workspace environment.
2. Commit Cargo, pnpm, and uv lockfiles. CI installs only from those locks.
3. Pull official upstream images through `m.daocloud.io`, with the upstream
   registry path retained in the image reference and the OCI index digest
   pinned.
4. Pin GitHub Actions to full commit SHAs. Version comments are informational.
5. Keep pnpm's 24-hour minimum package release age. New releases must mature
   before they can enter a regenerated lockfile.
6. Use one `just` interface locally and in CI while keeping language-native
   commands directly runnable for diagnosis.
7. In the China-hosted development environment, replace only the crates.io
   transport with the RSProxy sparse index. Cargo still enforces every version
   and checksum from `Cargo.lock`; publishing remains outside this repository's
   workflow.
8. Exclude secret-bearing environment files and local agent configuration from
   both Git and the Docker build context.
9. Treat `uv --locked` as the consistency gate. `--frozen` is not accepted for
   the active research package because it can use an outdated lock without
   reporting project drift.
10. Use official `https://deb.debian.org` package and security endpoints with
    Debian signature and Release-file expiration validation, bounded retries,
    and fatal index-fetch errors. This is independent of the DaoCloud transport
    used for OCI images. Amended on 2026-09-08 after the previously selected
    Tencent Cloud mirror served expired security metadata in CI run
    `34181369455`; validation must not be disabled to accommodate a stale mirror.
11. Bake the exact Clippy and rustfmt components into the development image.
    Install Cargo, rustc, rust-std, Clippy, and rustfmt from their individual
    upstream distribution archives. Verify every archive against the SHA-256
    manifest committed under `config/toolchains` before executing its installer.
    Runtime bootstrap reuses a toolchain only when its exact versions, resolved
    sysroot, and repository checksum marker all validate; an unmarked system or
    rustup toolchain is not treated as content-pinned.
12. Default host Rust component downloads to SJTUG. A closed
    `LOOP_ENGINE_RUST_DIST_MIRROR` selector also permits USTC, RSProxy, and the
    official distribution endpoint. Arbitrary mirror URLs are rejected. Mirrors
    are untrusted transports: the same repository-recorded component digests
    apply to every endpoint, and mismatched bytes fail before extraction.
13. Build the development image from the DaoCloud-proxied Python base and
    install those content-pinned Rust components directly. This avoids both the
    unusually large Rust OCI layer and rustup's second, separately fetched
    manifest while retaining pinned compiler, Cargo, Clippy, and rustfmt bytes.
14. Share the Rust component downloader between host bootstrap and the image
    build. After a transport failure, try the official endpoint and then RSProxy,
    skipping any already attempted source. Each source has a 10-second connection
    timeout and a 300-second total timeout, with no unbounded retry. A checksum
    mismatch is fatal, not a reason to try another source. Verified bytes replace
    a cache entry atomically; failed and interrupted downloads remove their
    uniquely named temporary files. GitHub-hosted CI selects the official source
    first; local development retains the SJTUG default. This amendment follows
    SJTUG connection failures in run `34330440769` and does not change OCI sources,
    component versions, digests, or verification gates.

## Consequences

- Image digest changes and toolchain upgrades require an explicit reviewed
  change.
- DaoCloud is a transport dependency, not the provenance authority. Image
  annotations and digests continue to identify the official upstream build.
- RSProxy is likewise a crate transport. It cannot change locked package bytes
  without failing Cargo's checksum verification.
- The Rust distribution mirror selector does not affect Docker/OCI image pulls; DaoCloud
  remains the separately controlled image transport.
- Rebuilding after an upstream registry outage remains possible while the
  pinned content is present in DaoCloud or an approved internal cache.
- The pinned base-image digest and language lockfiles provide identity and
  dependency reproducibility. The development image still installs Debian
  packages from the image's configured apt repository; it is not a claim of a
  byte-for-byte reproducible OCI image. A production image must use an approved
  package snapshot and emit an SBOM in Phase 14.
