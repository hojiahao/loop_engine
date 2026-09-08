# Container CI correction for 53a0ab2

- Date: 2026-09-08
- Owner: hojiahao
- Failed run: https://github.com/hojiahao/loop_engine/actions/runs/34181369455
- Failed job: `101920985264`, DaoCloud development container.

## Observed failure

The Rust, TypeScript, Python research, Python protocol, legacy regression, and
unified workspace jobs all passed. The container job downloaded its pinned
DaoCloud OCI images successfully, then failed during `apt-get update`.

The Tencent Cloud mirror's `bookworm-security/InRelease` had expired 5 hours,
36 minutes, and 25 seconds earlier. APT rejected it with exit status 100 before
the container reached application compilation or tests. This is a package
repository freshness failure, not a failed Rust test or OCI download.

## Correction and verification

- Use Debian's official HTTPS package and security endpoints.
- Preserve all DaoCloud image references and their pinned OCI digests.
- Keep Debian signature and Release-file expiration checks enabled.
- Make every failed index fetch fatal, including when older cached lists exist.
- Keep bounded APT retries and HTTPS timeouts.
- Local Compose configuration validation passed. Full clean-container and
  remote CI evidence must be recorded after the correction is pushed.

This correction does not complete Phase 3 or include the unfinished role-owned
submission and holdout transaction work.
