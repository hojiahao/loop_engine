# Phase 1 verification: reproducible polyglot toolchain

- Date: 2026-09-05 (Asia/Shanghai)
- Branch: `refactor/us-equities-loop-runtime`
- Implementation commit: pending remote push
- Scope: toolchains, workspace skeletons, unified gates, CI, and development container

## Pinned toolchains

| Tool | Verified version |
| --- | --- |
| Rust / Cargo | 1.93.1 |
| Clippy | 0.1.93 (`01f6ddf758`) |
| rustfmt | 1.8.0-stable (`01f6ddf758`) |
| Node.js | 24.17.0 |
| pnpm | 11.25.0 |
| Python | 3.12.13 |
| uv | 0.11.29 |
| just | 1.45.0 |

The Cargo, pnpm, legacy Python, and research Python locks were used without
modification. Tool versions and base-image references are recorded in source;
all four OCI bases use the DaoCloud transport and full upstream digests.

## Clean-container evidence

The development image was built from `.devcontainer/Dockerfile` after Docker
build caches were removed. Docker reported:

```text
image: loop-engine-development:latest
manifest-list: sha256:ec93815b4d0a3985fdc39b6487da3a513bd6726b431b656c2923ba2d30213d17
inspect-size: 473953731 bytes
```

An empty `development-runtime` volume was then used for every command below.
No host Python or Cargo environment was mounted into the container.
pnpm reported its content-addressable store as
`/var/lib/loop-engine-dev/.tools/pnpm-store/v11` and copy-imported packages into
the bind-mounted `node_modules`; `.pnpm-store` remained absent from the source
tree. `node_modules/.modules.yaml` recorded the same runtime-volume store path.

| Gate | Result |
| --- | --- |
| `./scripts/bootstrap.sh` | passed; exact versions above; locks installed from an empty volume |
| `just check` | passed before and after `just build` |
| `just test` | passed |
| `just build` | passed |
| `just doctor` | passed |

Test evidence:

- Rust workspace: 5 unit tests passed; all doc-test targets passed.
- TypeScript: Provider health contract test passed; the Phase 1 Web skeleton has
  no behavioral tests yet and exits explicitly with `--passWithNoTests`.
- Python research worker: 1 test passed.
- Legacy regression: 216 passed, 1 skipped, with the existing 12 NumPy
  degrees-of-freedom warnings in `test_orchestrate.py`.

Build evidence:

- all five Rust workspace members built from `Cargo.lock` in offline mode;
- providerd compiled with strict TypeScript and the React client built with Vite;
- `loop_research-0.2.0a1` sdist and wheel were created from its locked environment.

Doctor evidence:

```text
Loop Engine: loopctl is ready (0.2.0-alpha.1)
{"component":"researchd","protocol_version":"loop-engine.v1alpha1","status":"ready"}
providerd TypeScript typecheck: passed
```

The disposable runtime volume and Compose network were removed after the
verification. They contain only caches, virtual environments, and build output
and are recreated by `./scripts/bootstrap.sh`.

## Reproducibility boundary

This gate proves exact language tools, lock consistency, a digest-pinned base,
and clean-container execution. Debian packages still resolve from a signed,
moving mirror. Byte-identical production images, package snapshots, provenance
attestations, and SBOM policy remain explicit Phase 14 gates; this record makes
no stronger claim.
