# ADR 0005: Python 3.14 and one uv workspace environment

- Status: Accepted
- Date: 2026-09-07
- Owner: hojiahao
- Amends: ADR 0002 Python version and environment layout

## Context

Phase 1 pinned Python 3.12.13 and created separate virtual environments for the
legacy regression suite, `loop_research`, and `loop_protocol`. That layout
allowed the three lockfiles and installed tool versions to drift. Pointing
three independent uv projects at one environment is also unsafe because an
exact `uv sync` removes packages that are extraneous to the project currently
being synchronized.

The supported host now provides CPython 3.14.4. The locked numerical and
protocol dependencies used by Phases 1 and 2 publish compatible Linux x86-64
wheels. A toolchain migration can therefore be tested without changing factor
or accounting semantics.

## Decision

1. Pin the primary Python toolchain to CPython 3.14.4 in `.python-version`,
   bootstrap, CI, and the development image.
2. Make the repository root a uv workspace containing `loop_research` and
   `loop_protocol`. The frozen legacy dependencies remain on the virtual root
   project until Phase 13 quarantines that implementation.
3. Commit one root `uv.lock`. Every workspace member supports Python
   `>=3.14,<3.15`, whose intersection is recorded by uv as `==3.14.*`; the
   executable patch version remains exactly pinned by `.python-version` and
   bootstrap.
4. Use exactly one project environment at `<repository>/.venv`. The directory
   is ignored by Git and managed only through `scripts/uv.sh` or the `just`
   entry points. uv itself is an independently pinned executable and is not
   installed into the environment it manages.
5. `uv sync --all-packages --all-groups` installs the complete workspace in one
   transaction. Normal checks and tests run from the owning package directory
   but use the same locked root environment. Package-specific wrappers accept
   only `run` and pin the selected workspace package; all environment mutation
   goes through the root wrapper.
6. uv download and managed-interpreter caches may use the ignored runtime
   `.tools` directory or a `LOOP_ENGINE_RUNTIME_ROOT` volume. Changing the cache
   location never changes the `.venv` location.
7. The development container mounts a dedicated volume at
   `/workspace/.venv`. It therefore uses the same workspace-relative path while
   preventing host interpreter paths from entering the container environment.
   The container exit gate uses a unique Compose project for every invocation,
   then removes its volumes even when a command fails.
8. Wire fixture producers read the Python patch pin from `.python-version`.
   Changing it requires fixture regeneration and the complete cross-language
   compatibility gate; it does not advance the Protobuf baseline.
9. Python 3.14 compatibility changes are made only when lint, type, or behavior
   tests expose a concrete issue. Numerical code is not mechanically rewritten
   during a toolchain upgrade.

## Independent validator boundary

Phase 8 must test Alphalens Reloaded and Zipline Reloaded against Python 3.14
before adding either lock. If an independent validator cannot support 3.14, it
uses a separately versioned process and lockfile. That process exchanges only
immutable artifact references, schemas, and SHA-256 digests with
`loop_research`; it never becomes an import-time dependency of the primary
research environment.

## Consequences

- IDEs and shell tools discover the conventional root `.venv` automatically.
- One lock and one exact sync remove cross-environment dependency drift.
- The combined development environment is larger, so package CI tests use uv's
  ephemeral `--isolated` environment to prove that each package's tests pass
  with only its selected dependency closure. This is not another persistent
  project `.venv`.
- The Phase 1 Python 3.12.13 verification remains historical evidence. This
  amendment requires its own host, container, and remote CI evidence before it
  can be marked complete.

## Verification

The amendment exit gate requires:

1. `uv lock --check` and an exact standard CPython 3.14.4 environment check;
2. proof that the repository contains only the root `.venv`;
3. protocol fixture regeneration plus all cross-language contract tests;
4. `just check`, `just test`, `just build`, and `just doctor` on the host;
5. the legacy 216 passed / 1 skipped regression result;
6. a clean DaoCloud development-container run under a unique Compose project
   with fresh `.venv` and runtime volumes; and
7. a successful pushed GitHub Actions run.

## References

- [uv workspaces](https://docs.astral.sh/uv/concepts/projects/workspaces/)
- [uv project environments](https://docs.astral.sh/uv/concepts/projects/config/#project-environment-path)
- [Python 3.14.4 slim-bookworm image](https://hub.docker.com/layers/library/python/3.14.4-slim-bookworm/images/sha256-db5942d111df72110e7a67da3fc5159e83ac85cd24808b91bdc4769e166ed1b7)
