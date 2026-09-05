# ADR 0002: Reproducible polyglot toolchain

- Status: Accepted
- Date: 2026-09-04
- Owners: hojiahao

## Context

Loop Engine spans a Rust control plane, a TypeScript provider/UI plane, and a
Python research plane. Floating language versions and unpinned container or CI
inputs would make financial results and operational failures difficult to
reproduce.

Direct Docker Hub access is not reliable in the deployment environment. The
operator requires DaoCloud as the image transport.

## Decision

1. Pin Rust 1.93.1, Node 24.17.0, pnpm 11.25.0, Python 3.12.13, uv 0.11.29,
   and just 1.45.0.
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
10. Use the Tencent Cloud HTTPS Debian mirror with normal Debian signature validation
    and bounded apt retries in the development image. This is independent of
    the DaoCloud transport used for OCI images.
11. Bake the exact Clippy and rustfmt components into the development image.
    rustup verifies their signed toolchain manifests and downloads them through
    RSProxy during image construction; runtime bootstrap must not reinstall a
    second Rust toolchain when the pinned system toolchain is complete.

## Consequences

- Image digest changes and toolchain upgrades require an explicit reviewed
  change.
- DaoCloud is a transport dependency, not the provenance authority. Image
  annotations and digests continue to identify the official upstream build.
- RSProxy is likewise a crate transport. It cannot change locked package bytes
  without failing Cargo's checksum verification.
- Rebuilding after an upstream registry outage remains possible while the
  pinned content is present in DaoCloud or an approved internal cache.
- The pinned base-image digest and language lockfiles provide identity and
  dependency reproducibility. The development image still installs Debian
  packages from the image's configured apt repository; it is not a claim of a
  byte-for-byte reproducible OCI image. A production image must use an approved
  package snapshot and emit an SBOM in Phase 14.
